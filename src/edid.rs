//! Minimal EDID parser: physical image size and model name.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edid {
    /// Visible image size in millimetres (width, height).
    pub size_mm: Option<(f64, f64)>,
    pub model: String,
}

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

pub fn parse(data: &[u8]) -> Option<Edid> {
    if data.len() < 128 || data[..8] != HEADER {
        return None;
    }
    let cm = (data[21] as f64 * 10.0, data[22] as f64 * 10.0);
    let mut dtd_mm = None;
    let mut model = String::new();

    for d in data[54..126].as_chunks::<18>().0 {
        if d[0] != 0 || d[1] != 0 {
            // Detailed timing descriptor; the first one is the preferred mode.
            if dtd_mm.is_none() {
                let w = d[12] as u32 | ((d[14] as u32 & 0xF0) << 4);
                let h = d[13] as u32 | ((d[14] as u32 & 0x0F) << 8);
                dtd_mm = Some((w as f64, h as f64));
            }
        } else if d[3] == 0xFC {
            model = d[5..18]
                .iter()
                .take_while(|&&b| b != 0x0A && b != 0)
                .map(|&b| b as char)
                .collect::<String>()
                .trim()
                .to_string();
        }
    }

    Some(Edid { size_mm: pick_size(dtd_mm, cm), model })
}

/// The descriptor gives millimetres but some panels put an aspect ratio
/// (16x9) or zeros there; the header gives whole centimetres. Prefer the
/// precise value when it agrees with the coarse one.
fn pick_size(dtd: Option<(f64, f64)>, cm: (f64, f64)) -> Option<(f64, f64)> {
    let plausible = |(w, h): (f64, f64)| w >= 100.0 && h >= 50.0 && w < 5000.0 && h < 5000.0;
    let cm_ok = plausible(cm);
    match dtd {
        Some(mm) if plausible(mm) && (!cm_ok || ((mm.0 - cm.0).abs() <= 15.0 && (mm.1 - cm.1).abs() <= 15.0)) => {
            Some(mm)
        }
        _ if cm_ok => Some(cm),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(cm: (u8, u8), mm: (u16, u16), name: &str) -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&HEADER);
        e[21] = cm.0;
        e[22] = cm.1;
        // DTD at 54
        e[54] = 0x02;
        e[55] = 0x3A;
        e[54 + 12] = (mm.0 & 0xFF) as u8;
        e[54 + 13] = (mm.1 & 0xFF) as u8;
        e[54 + 14] = (((mm.0 >> 8) as u8) << 4) | ((mm.1 >> 8) as u8 & 0x0F);
        // Name at 72
        e[72 + 3] = 0xFC;
        let mut n = name.as_bytes().to_vec();
        n.push(0x0A);
        n.resize(13, 0x20);
        e[72 + 5..72 + 18].copy_from_slice(&n);
        e
    }

    #[test]
    fn reads_mm_and_name() {
        let e = parse(&sample((60, 34), (597, 336), "LG QHD")).unwrap();
        assert_eq!(e.size_mm, Some((597.0, 336.0)));
        assert_eq!(e.model, "LG QHD");
    }

    #[test]
    fn aspect_placeholder_falls_back_to_cm() {
        let e = parse(&sample((53, 30), (16, 9), "X")).unwrap();
        assert_eq!(e.size_mm, Some((530.0, 300.0)));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(&[0u8; 128]).is_none());
        assert_eq!(parse(&sample((0, 0), (0, 0), "")).unwrap().size_mm, None);
    }
}
