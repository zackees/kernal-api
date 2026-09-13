use std::sync::OnceLock;

/// Physical CPU cores on this machine, or `None` when the topology could
/// not be read. Memoized: the daemon asks once at startup.
pub fn physical_cores() -> Option<usize> {
    static CACHED: OnceLock<Option<usize>> = OnceLock::new();
    *CACHED.get_or_init(|| detect_cores().filter(|cores| *cores > 0))
}

/// `GetLogicalProcessorInformationEx(RelationProcessorCore, ..)` returns
/// exactly one variable-length record per physical core, so counting
/// records is the answer.
fn detect_cores() -> Option<usize> {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
    };

    // Records are 8-byte aligned; backing the buffer with `u64` gives
    // that alignment for free, and the header fields are still read
    // unaligned so a driver reporting an odd `Size` cannot cause UB.
    const HEADER_BYTES: usize = 8; // Relationship: u32, Size: u32

    let mut len: u32 = 0;
    // The sizing call is *expected* to fail with
    // ERROR_INSUFFICIENT_BUFFER; only `len` matters here.
    unsafe {
        GetLogicalProcessorInformationEx(RelationProcessorCore, std::ptr::null_mut(), &mut len)
    };
    if (len as usize) < HEADER_BYTES {
        return None;
    }
    let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
    let ok = unsafe {
        GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            buffer.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if ok == 0 {
        return None;
    }

    let base = buffer.as_ptr().cast::<u8>();
    let len = len as usize;
    let mut offset = 0usize;
    let mut cores = 0usize;
    while offset + HEADER_BYTES <= len {
        // SAFETY: `offset + HEADER_BYTES <= len` and `buffer` holds at
        // least `len` bytes, so both reads are in bounds. Unaligned
        // reads have no alignment requirement.
        let (relationship, size) = unsafe {
            let record = base.add(offset);
            (
                record.cast::<u32>().read_unaligned(),
                record.add(4).cast::<u32>().read_unaligned() as usize,
            )
        };
        // A zero or out-of-range `Size` would loop forever or walk off
        // the buffer. Stop and report what was counted so far.
        if size < HEADER_BYTES || offset + size > len {
            break;
        }
        if relationship == RelationProcessorCore as u32 {
            cores += 1;
        }
        offset += size;
    }
    (cores > 0).then_some(cores)
}
