//! The update check (2.02): which release is the latest on GitHub, through WinHTTP (Windows' own HTTP client, with
//! the system's proxy settings). github.com/<repo>/releases/latest answers with a redirect to /releases/tag/<tag>;
//! when it doesn't, the API's releases/latest says it in its JSON (the API allows 60 asks an hour per address, so it
//! only backs up the first).

use std::ffi::c_void;

use windows::Win32::Foundation::GetLastError;
use windows::Win32::Networking::WinHttp::*;
use windows::core::PCWSTR;

/// A WinHTTP handle, closed when dropped.
struct H(*mut c_void);

impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn failed(what: &str) -> String {
    let code = unsafe { GetLastError() }.0;
    match code {
        12002 => format!("{what}: no answer in time"),
        12007 => format!("{what}: the server's name couldn't be found (offline?)"),
        12029 => format!("{what}: couldn't connect (offline?)"),
        _ => format!("{what}: WinHTTP error {code}"),
    }
}

/// One HTTPS request that doesn't follow redirects: (status, the Location header, the body up to `max` bytes).
fn request(host: &str, path: &str, verb: &str, max: usize) -> Result<(u32, String, Vec<u8>), String> {
    unsafe {
        let agent = wide(&format!("Choppy/{}", env!("CHOPPY_VERSION")));
        let s = H(WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if s.0.is_null() {
            return Err(failed("WinHttpOpen"));
        }
        let _ = WinHttpSetTimeouts(s.0, 10_000, 10_000, 10_000, 15_000);
        let host_w = wide(host);
        let c = H(WinHttpConnect(s.0, PCWSTR(host_w.as_ptr()), 443, 0));
        if c.0.is_null() {
            return Err(failed(host));
        }
        let (verb_w, path_w) = (wide(verb), wide(path));
        let r = H(WinHttpOpenRequest(c.0, PCWSTR(verb_w.as_ptr()), PCWSTR(path_w.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if r.0.is_null() {
            return Err(failed(host));
        }
        let off = WINHTTP_DISABLE_REDIRECTS.to_le_bytes();
        let _ = WinHttpSetOption(Some(r.0), WINHTTP_OPTION_DISABLE_FEATURE, Some(&off));
        if WinHttpSendRequest(r.0, None, None, 0, 0, 0).is_err() || WinHttpReceiveResponse(r.0, std::ptr::null_mut()).is_err() {
            return Err(failed(host));
        }
        let mut status = 0u32;
        let mut n = 4u32;
        if WinHttpQueryHeaders(
            r.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut n,
            std::ptr::null_mut(),
        )
        .is_err()
        {
            return Err(failed(host));
        }
        let mut loc = vec![0u16; 1024];
        let mut n = (loc.len() * 2) as u32;
        let location = match WinHttpQueryHeaders(r.0, WINHTTP_QUERY_LOCATION, PCWSTR::null(), Some(loc.as_mut_ptr() as *mut c_void), &mut n, std::ptr::null_mut()) {
            Ok(()) => String::from_utf16_lossy(&loc[..(n as usize / 2).min(loc.len())]),
            Err(_) => String::new(),
        };
        let mut body = Vec::new();
        while body.len() < max {
            let mut avail = 0u32;
            if WinHttpQueryDataAvailable(r.0, &mut avail).is_err() || avail == 0 {
                break;
            }
            let mut chunk = vec![0u8; (avail as usize).min(max - body.len())];
            let mut got = 0u32;
            if WinHttpReadData(r.0, chunk.as_mut_ptr() as *mut c_void, chunk.len() as u32, &mut got).is_err() || got == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..got as usize]);
        }
        Ok((status, location, body))
    }
}

/// The latest release's tag of `repo` ("owner/name"), as GitHub names it ("v2.02").
pub fn latest_release_tag(repo: &str) -> Result<String, String> {
    let first = match request("github.com", &format!("/{repo}/releases/latest"), "HEAD", 0) {
        Ok((status, location, _)) => match location.split("/releases/tag/").nth(1) {
            Some(tag) if (300..400).contains(&status) => return Ok(tag.trim_end_matches('/').to_string()),
            _ => format!("github.com answered {status} without a release"),
        },
        Err(e) => e,
    };
    let (status, _, body) = request("api.github.com", &format!("/repos/{repo}/releases/latest"), "GET", 512 * 1024).map_err(|e| format!("{first}; {e}"))?;
    let text = String::from_utf8_lossy(&body);
    // "tag_name": "v2.02"
    let tag = text
        .split_once("\"tag_name\"")
        .and_then(|(_, rest)| rest.trim_start().strip_prefix(':'))
        .and_then(|rest| rest.trim_start().strip_prefix('"'))
        .and_then(|rest| rest.split_once('"'))
        .map(|(t, _)| t.to_string());
    tag.ok_or_else(|| format!("{first}; api.github.com answered {status} without a release"))
}
