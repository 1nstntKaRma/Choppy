//! Choppy.ico: the window / taskbar icons (HICON) and the title-bar image (ChoppyIcon.Frame: the smallest frame at
//! least as big as the pixels needed, scaled with WIC's Fant filter like WPF's BitmapScalingMode.HighQuality).

use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

pub const ICO: &[u8] = include_bytes!("../../../../src/Choppy.ico");

/// (width, offset, size) of each image in the .ico.
fn entries() -> Vec<(u32, usize, usize)> {
    let n = u16::from_le_bytes([ICO[4], ICO[5]]) as usize;
    (0..n)
        .map(|i| {
            let e = &ICO[6 + i * 16..6 + i * 16 + 16];
            let w = if e[0] == 0 { 256 } else { e[0] as u32 };
            let size = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
            let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
            (w, off, size)
        })
        .collect()
}

/// The smallest image at least `px` wide (else the largest).
fn best(px: u32) -> (u32, usize, usize) {
    let all = entries();
    all.iter().filter(|e| e.0 >= px).min_by_key(|e| e.0).copied().unwrap_or_else(|| *all.iter().max_by_key(|e| e.0).unwrap())
}

pub fn hicon(px: u32) -> Option<HICON> {
    let (_, off, size) = best(px);
    unsafe { CreateIconFromResourceEx(&ICO[off..off + size], true, 0x0003_0000, px as i32, px as i32, LR_DEFAULTCOLOR).ok() }
}

/// The title-bar image at exactly `px` x `px`, as a Direct2D bitmap for `rt`.
pub fn bitmap(rt: &ID2D1RenderTarget, px: u32) -> Option<ID2D1Bitmap> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = SHCreateMemStream(Some(ICO))?;
        let dec = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let n = dec.GetFrameCount().ok()?;
        let mut pick: Option<(u32, IWICBitmapFrameDecode)> = None;
        let mut largest: Option<(u32, IWICBitmapFrameDecode)> = None;
        for i in 0..n {
            let f = dec.GetFrame(i).ok()?;
            let (mut w, mut h) = (0, 0);
            f.GetSize(&mut w, &mut h).ok()?;
            if largest.as_ref().is_none_or(|l| w > l.0) {
                largest = Some((w, f.clone()));
            }
            if w >= px && pick.as_ref().is_none_or(|p| w < p.0) {
                pick = Some((w, f));
            }
        }
        let (w, frame) = pick.or(largest)?;
        let conv = wic.CreateFormatConverter().ok()?;
        conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        let src: IWICBitmapSource = if w != px {
            let sc = wic.CreateBitmapScaler().ok()?;
            sc.Initialize(&conv, px, px, WICBitmapInterpolationModeFant).ok()?;
            sc.cast().ok()?
        } else {
            conv.cast().ok()?
        };
        rt.CreateBitmapFromWicBitmap(&src, None).ok()
    }
}
