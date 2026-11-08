//! Spelling for raw Unicode Win32 file APIs. This is lexical conversion only:
//! it confers no ownership, ACL, no-reparse or confinement authority.
const SEP: u16 = b'\\' as u16;
const VERBATIM: [u16; 4] = [SEP, SEP, b'?' as u16, SEP];
const UNC: [u16; 8] = [SEP, SEP, b'?' as u16, SEP, b'U' as u16, b'N' as u16, b'C' as u16, SEP];

/// Input is an absolute Windows path normalized by std::path::absolute.
/// Keep UTF-16 code units, including valid Windows names with unpaired
/// surrogates, rather than round-tripping through a lossy Rust String.
pub(super) fn encode_absolute(path: &[u16]) -> Option<Vec<u16>> {
    if path.is_empty() || path.contains(&0) {
        return None;
    }
    let mut result = if path.starts_with(&VERBATIM) {
        if path.len() == VERBATIM.len() {
            return None;
        }
        path.to_vec()
    } else {
        if path.contains(&(b'/' as u16)) {
            return None;
        }
        let drive = path.len() >= 3
            && u8::try_from(path[0]).ok().is_some_and(|letter| letter.is_ascii_alphabetic())
            && path[1] == b':' as u16
            && path[2] == SEP;
        let unc = path.starts_with(&[SEP, SEP]);
        let tail = if drive {
            &path[3..]
        } else if unc {
            &path[2..]
        } else {
            return None;
        };
        let components: Vec<_> = tail.split(|unit| *unit == SEP).collect();
        if unc && (components.len() < 2 || components[0].is_empty() || components[1].is_empty()) {
            return None;
        }
        for (index, component) in components.iter().enumerate() {
            // Reject ordinary spellings whose Win32 normalization would change
            // meaning after adding a verbatim prefix (including ADS/device
            // namespace forms). A final separator is permitted for a root.
            if component.is_empty() && index + 1 != components.len()
                || component.last().is_some_and(|last| *last == b'.' as u16 || *last == b' ' as u16)
                || component.contains(&(b':' as u16))
                || (!unc || index >= 2) && ordinary_device(component)
            {
                return None;
            }
        }
        let mut result = if drive { VERBATIM.to_vec() } else { UNC.to_vec() };
        result.extend_from_slice(if drive { path } else { &path[2..] });
        result
    };
    // Include the terminator in the native path bound.
    if result.len() >= 32_767 {
        return None;
    }
    result.push(0);
    Some(result)
}

fn ordinary_device(component: &[u16]) -> bool {
    let stem = component.split(|unit| *unit == b'.' as u16).next().unwrap_or_default();
    if stem.len() > 7 {
        return false;
    }
    let upper: Vec<u16> = stem
        .iter()
        .map(|unit| u8::try_from(*unit).map_or(*unit, |ascii| u16::from(ascii.to_ascii_uppercase())))
        .collect();
    let is = |name: &str| upper.iter().copied().eq(name.encode_utf16());
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].into_iter().any(is) {
        return true;
    }
    upper.len() == 4
        && (upper[..3].iter().copied().eq("COM".encode_utf16()) || upper[..3].iter().copied().eq("LPT".encode_utf16()))
        && matches!(upper[3], 49..=57 | 0xb9 | 0xb2 | 0xb3)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn units(path: &str) -> Vec<u16> {
        path.encode_utf16().collect()
    }

    #[test]
    fn private_store_windows_paths_extend_long_drive_unc_and_preserve_verbatim_utf16() {
        for (plain, extended) in [
            (r"C:\profiles", r"\\?\C:\profiles"),
            (r"\\server\share\profiles", r"\\?\UNC\server\share\profiles"),
        ] {
            let tail = format!("\\{}\\.native-store-00000000-0000-0000-0000-000000000000.tmp", "x".repeat(280));
            let actual = encode_absolute(&units(&format!("{plain}{tail}"))).unwrap();
            let mut expected = units(&format!("{extended}{tail}"));
            expected.push(0);
            assert_eq!(actual, expected);
            assert_eq!(encode_absolute(&actual[..actual.len() - 1]).unwrap(), expected);
        }
        let mut unusual = units(r"C:\private\");
        unusual.extend([0xd800, 0xdc01, 0xdfff]);
        let encoded = encode_absolute(&unusual).unwrap();
        assert_eq!(&encoded[4..encoded.len() - 1], unusual.as_slice());
    }
    #[test]
    fn private_store_windows_paths_refuse_relative_alias_device_and_nul_spellings() {
        for path in [
            "",
            r"relative\file",
            r"C:file",
            r"\rooted",
            r"\\.\pipe\name",
            r"\\server",
            r"\\server\\share",
            r"C:\foo\..\bar",
            r"C:\foo.\bar",
            r"C:\foo \bar",
            r"C:\file:stream",
            "C:/foo/bar",
            r"C:\private\NUL",
            r"C:\private\CON.txt",
            r"C:\private\com1.bin",
            r"\\server\share\LPT1",
            r"C:\private\COM².log",
            r"C:\private\CONOUT$",
        ] {
            assert!(encode_absolute(&units(path)).is_none(), "{path:?}");
        }
        let mut nul = units(r"C:\private\file");
        nul.push(0);
        assert!(encode_absolute(&nul).is_none());
        assert_eq!(encode_absolute(&units(r"C:\")).unwrap(), units("\\\\?\\C:\\\0"));
    }
    #[test]
    fn private_store_windows_paths_enforce_the_complete_terminated_native_bound() {
        let base = units(r"\\?\C:\");
        let mut largest = base.clone();
        largest.resize(32_766, b'x' as u16);
        assert_eq!(encode_absolute(&largest).unwrap().len(), 32_767);
        largest.push(b'x' as u16);
        assert!(encode_absolute(&largest).is_none());
    }
}
