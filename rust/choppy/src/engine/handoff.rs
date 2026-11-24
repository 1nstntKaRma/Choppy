//! The administrator hand-off (Handoff.cs): step 2. Only what step 1's Journal already calls is here.

/// Administrator copies whose Choppy is gone: their files are removed (Handoff.CleanStale). Step 2 ports it; until then
/// nothing is removed, which is what an install without administrator copies sees anyway.
pub fn clean_stale() {}
