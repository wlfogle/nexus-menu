//! Reading the target of a Windows `.lnk` shortcut.
//!
//! Only one thing is needed from a shortcut: the path of the program it
//! points to, which a launcher such as PortProton or Faugus must be handed
//! because it cannot run the shortcut itself. The layout is the Shell Link
//! Binary File Format (MS-SHLLINK): a 76-byte header, an optional target ID
//! list, then the *LinkInfo* block whose local base path is the target.
//!
//! Shortcuts that carry no LinkInfo, such as ones that point at a shell
//! folder or at an installer, have no readable target and yield `None`.

const HEADER_SIZE: u32 = 0x4C;
const HAS_LINK_TARGET_ID_LIST: u32 = 0x1;
const HAS_LINK_INFO: u32 = 0x2;
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 0x1;
/// LinkInfo headers at least this long carry Unicode path offsets as well.
const UNICODE_HEADER_SIZE: usize = 0x24;

fn u16_at(bytes: &[u8], at: usize) -> Option<usize> {
    let s = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([s[0], s[1]]) as usize)
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let s = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// A NUL-terminated single-byte string starting at `at`.
fn ansi_at(bytes: &[u8], at: usize) -> Option<String> {
    let tail = bytes.get(at..)?;
    let end = tail.iter().position(|&b| b == 0)?;
    Some(String::from_utf8_lossy(&tail[..end]).into_owned())
}

/// A NUL-terminated UTF-16LE string starting at `at`.
fn utf16_at(bytes: &[u8], at: usize) -> Option<String> {
    let tail = bytes.get(at..)?;
    let mut units = Vec::new();
    for pair in tail.chunks_exact(2) {
        let unit = u16::from_le_bytes([pair[0], pair[1]]);
        if unit == 0 {
            return Some(String::from_utf16_lossy(&units));
        }
        units.push(unit);
    }
    None
}

/// The Windows path a shortcut points to, such as
/// `C:\Program Files\NTLite\NTLite.exe`.
pub fn target(bytes: &[u8]) -> Option<String> {
    if u32_at(bytes, 0)? != HEADER_SIZE {
        return None;
    }
    let flags = u32_at(bytes, 20)?;

    let mut pos = HEADER_SIZE as usize;
    if flags & HAS_LINK_TARGET_ID_LIST != 0 {
        pos = pos.checked_add(2 + u16_at(bytes, pos)?)?;
    }
    if flags & HAS_LINK_INFO == 0 {
        return None;
    }

    // Offsets inside LinkInfo are relative to its first byte.
    let info = pos;
    let header_len = u32_at(bytes, info + 4)? as usize;
    if u32_at(bytes, info + 8)? & VOLUME_ID_AND_LOCAL_BASE_PATH == 0 {
        return None;
    }

    let ansi_base = u32_at(bytes, info + 16)? as usize;
    let ansi_suffix = u32_at(bytes, info + 24)? as usize;
    let (base, suffix) = if header_len >= UNICODE_HEADER_SIZE {
        let unicode_base = u32_at(bytes, info + 28)? as usize;
        let unicode_suffix = u32_at(bytes, info + 32)? as usize;
        if unicode_base != 0 {
            (
                utf16_at(bytes, info + unicode_base)?,
                utf16_at(bytes, info + unicode_suffix).unwrap_or_default(),
            )
        } else {
            (
                ansi_at(bytes, info + ansi_base)?,
                ansi_at(bytes, info + ansi_suffix).unwrap_or_default(),
            )
        }
    } else {
        (
            ansi_at(bytes, info + ansi_base)?,
            ansi_at(bytes, info + ansi_suffix).unwrap_or_default(),
        )
    };

    if base.is_empty() {
        return None;
    }
    if suffix.is_empty() {
        Some(base)
    } else if base.ends_with('\\') {
        Some(format!("{base}{suffix}"))
    } else {
        Some(format!("{base}\\{suffix}"))
    }
}

/// Build a minimal valid shortcut pointing at `target`, for tests.
#[cfg(test)]
pub fn fixture(target: &str) -> Vec<u8> {
    let mut out = header(HAS_LINK_INFO);
    out.extend(link_info_ansi(target, ""));
    out
}

#[cfg(test)]
fn header(flags: u32) -> Vec<u8> {
    let mut h = vec![0u8; HEADER_SIZE as usize];
    h[0..4].copy_from_slice(&HEADER_SIZE.to_le_bytes());
    h[20..24].copy_from_slice(&flags.to_le_bytes());
    h
}

#[cfg(test)]
fn link_info_ansi(base: &str, suffix: &str) -> Vec<u8> {
    let header_len = 28u32;
    let base_off = header_len;
    let suffix_off = base_off + base.len() as u32 + 1;
    let size = suffix_off + suffix.len() as u32 + 1;

    let mut v = Vec::new();
    for word in [
        size,
        header_len,
        VOLUME_ID_AND_LOCAL_BASE_PATH,
        header_len,
        base_off,
        0,
        suffix_off,
    ] {
        v.extend(word.to_le_bytes());
    }
    v.extend(base.bytes());
    v.push(0);
    v.extend(suffix.bytes());
    v.push(0);
    v
}

#[cfg(test)]
fn link_info_unicode(base: &str) -> Vec<u8> {
    let header_len = 36u32;
    // Placeholder single-byte strings, then the UTF-16 ones.
    let ansi_base = header_len;
    let ansi_suffix = ansi_base + 2;
    let unicode_base = ansi_suffix + 1;
    let units: Vec<u16> = base.encode_utf16().collect();
    let unicode_suffix = unicode_base + (units.len() as u32 + 1) * 2;
    let size = unicode_suffix + 2;

    let mut v = Vec::new();
    for word in [
        size,
        header_len,
        VOLUME_ID_AND_LOCAL_BASE_PATH,
        header_len,
        ansi_base,
        0,
        ansi_suffix,
        unicode_base,
        unicode_suffix,
    ] {
        v.extend(word.to_le_bytes());
    }
    v.extend([b'?', 0]); // ANSI base
    v.push(0); // ANSI suffix
    for unit in units {
        v.extend(unit.to_le_bytes());
    }
    v.extend([0, 0]); // end of UTF-16 base
    v.extend([0, 0]); // empty UTF-16 suffix
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_ansi_target() {
        let bytes = fixture(r"C:\Program Files\NTLite\NTLite.exe");
        assert_eq!(
            target(&bytes).as_deref(),
            Some(r"C:\Program Files\NTLite\NTLite.exe")
        );
    }

    #[test]
    fn skips_a_target_id_list_before_the_link_info() {
        let mut bytes = header(HAS_LINK_TARGET_ID_LIST | HAS_LINK_INFO);
        let id_list = [0xAAu8; 10];
        bytes.extend((id_list.len() as u16).to_le_bytes());
        bytes.extend(id_list);
        bytes.extend(link_info_ansi(r"D:\Games\Game.exe", ""));
        assert_eq!(target(&bytes).as_deref(), Some(r"D:\Games\Game.exe"));
    }

    #[test]
    fn prefers_the_unicode_path_when_present() {
        let mut bytes = header(HAS_LINK_INFO);
        bytes.extend(link_info_unicode("C:\\Games\\Pok\u{e9}mon Z-A\\game.exe"));
        assert_eq!(
            target(&bytes).as_deref(),
            Some("C:\\Games\\Pok\u{e9}mon Z-A\\game.exe")
        );
    }

    #[test]
    fn joins_a_common_path_suffix_to_the_base() {
        let mut bytes = header(HAS_LINK_INFO);
        bytes.extend(link_info_ansi(r"C:\Games\X", "x.exe"));
        assert_eq!(target(&bytes).as_deref(), Some(r"C:\Games\X\x.exe"));

        let mut with_slash = header(HAS_LINK_INFO);
        with_slash.extend(link_info_ansi(r"C:\Games\X\", "x.exe"));
        assert_eq!(target(&with_slash).as_deref(), Some(r"C:\Games\X\x.exe"));
    }

    #[test]
    fn unreadable_shortcuts_have_no_target() {
        // Not a shortcut at all.
        assert_eq!(target(b""), None);
        assert_eq!(target(b"MZ not a link"), None);

        // Wrong header size.
        let mut bad = fixture(r"C:\a.exe");
        bad[0] = 0x10;
        assert_eq!(target(&bad), None);

        // No LinkInfo block: nothing to read.
        assert_eq!(target(&header(0)), None);
        assert_eq!(target(&header(HAS_LINK_TARGET_ID_LIST)), None);

        // LinkInfo without a local base path.
        let mut no_local = fixture(r"C:\a.exe");
        no_local[HEADER_SIZE as usize + 8] = 0;
        assert_eq!(target(&no_local), None);

        // Truncated anywhere inside the structure.
        let full = fixture(r"C:\Program Files\App\app.exe");
        for len in 0..full.len() - 1 {
            assert_eq!(target(&full[..len]), None, "truncated to {len} bytes");
        }
    }
}
