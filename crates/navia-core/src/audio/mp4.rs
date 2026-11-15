//! Reads gapless info from MP4 atoms. Symphonia ignores edit lists and iTunSMPB, so AAC in M4A
//! needs this to play gaplessly.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::audio::decode::ITunSmpb;

#[derive(Clone, Copy, Debug, Default)]
pub struct Mp4Gapless {
    pub smpb: Option<ITunSmpb>,
    /// Media time in track timescale units, and segment duration in movie timescale units.
    pub edit: Option<(i64, u64)>,
    pub movie_timescale: u32,
    pub track_timescale: u32,
}

impl Mp4Gapless {
    /// Frames to skip at the start and frames to play, when the file says so.
    pub fn trim(&self) -> Option<(u64, Option<u64>)> {
        if let Some(s) = self.smpb {
            return Some((s.priming, Some(s.valid)));
        }
        let (media_time, duration) = self.edit?;
        if self.movie_timescale == 0 || self.track_timescale == 0 {
            return None;
        }
        let frames = duration as u128 * self.track_timescale as u128 / self.movie_timescale as u128;
        Some((media_time.max(0) as u64, Some(frames as u64)))
    }
}

/// Atom cursor over a byte buffer holding the `moov` atom.
fn atoms(buf: &[u8], mut pos: usize, end: usize) -> impl Iterator<Item = ([u8; 4], usize, usize)> + '_ {
    std::iter::from_fn(move || {
        if pos + 8 > end {
            return None;
        }
        let mut size = u32::from_be_bytes(buf[pos..pos + 4].try_into().ok()?) as usize;
        let kind: [u8; 4] = buf[pos + 4..pos + 8].try_into().ok()?;
        let mut header = 8;
        if size == 1 {
            size = u64::from_be_bytes(buf[pos + 8..pos + 16].try_into().ok()?) as usize;
            header = 16;
        } else if size == 0 {
            size = end - pos;
        }
        if size < header || pos + size > end {
            return None;
        }
        let item = (kind, pos + header, pos + size);
        pos += size;
        Some(item)
    })
}

fn child(buf: &[u8], (start, end): (usize, usize), kind: &[u8; 4]) -> Option<(usize, usize)> {
    atoms(buf, start, end).find(|(k, _, _)| k == kind).map(|(_, a, b)| (a, b))
}

fn path(buf: &[u8], mut range: (usize, usize), kinds: &[&[u8; 4]]) -> Option<(usize, usize)> {
    for kind in kinds {
        range = child(buf, range, kind)?;
    }
    Some(range)
}

fn read_moov<R: Read + Seek + ?Sized>(f: &mut R) -> Option<Vec<u8>> {
    let len = f.seek(SeekFrom::End(0)).ok()?;
    let mut pos = 0u64;
    while pos + 8 <= len {
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut head = [0u8; 16];
        f.read_exact(&mut head[..8]).ok()?;
        let mut size = u32::from_be_bytes(head[..4].try_into().ok()?) as u64;
        let mut header = 8u64;
        if size == 1 {
            f.read_exact(&mut head[8..16]).ok()?;
            size = u64::from_be_bytes(head[8..16].try_into().ok()?);
            header = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < header {
            return None;
        }
        if &head[4..8] == b"moov" {
            let mut buf = vec![0u8; (size - header) as usize];
            f.seek(SeekFrom::Start(pos + header)).ok()?;
            f.read_exact(&mut buf).ok()?;
            return Some(buf);
        }
        pos += size;
    }
    None
}

pub fn read(file: &Path) -> Option<Mp4Gapless> {
    read_from(&mut File::open(file).ok()?)
}

/// Read gapless data from any seekable source.
pub fn read_from<R: Read + Seek + ?Sized>(file: &mut R) -> Option<Mp4Gapless> {
    let buf = read_moov(file)?;
    let root = (0, buf.len());
    let mut out = Mp4Gapless::default();

    let mvhd = child(&buf, root, b"mvhd")?;
    let version = buf[mvhd.0];
    let offset = if version == 1 { 20 } else { 12 };
    out.movie_timescale = u32::from_be_bytes(buf[mvhd.0 + offset..mvhd.0 + offset + 4].try_into().ok()?);

    for (kind, a, b) in atoms(&buf, root.0, root.1) {
        if &kind != b"trak" {
            continue;
        }
        let mdia = child(&buf, (a, b), b"mdia")?;
        let hdlr = child(&buf, mdia, b"hdlr")?;
        if &buf[hdlr.0 + 8..hdlr.0 + 12] != b"soun" {
            continue;
        }
        let mdhd = child(&buf, mdia, b"mdhd")?;
        let v = buf[mdhd.0];
        let offset = if v == 1 { 20 } else { 12 };
        out.track_timescale = u32::from_be_bytes(buf[mdhd.0 + offset..mdhd.0 + offset + 4].try_into().ok()?);
        if let Some(elst) = path(&buf, (a, b), &[b"edts", b"elst"]) {
            let v = buf[elst.0];
            let count = u32::from_be_bytes(buf[elst.0 + 4..elst.0 + 8].try_into().ok()?);
            if count == 1 {
                let e = elst.0 + 8;
                out.edit = Some(if v == 1 {
                    let d = u64::from_be_bytes(buf[e..e + 8].try_into().ok()?);
                    let m = i64::from_be_bytes(buf[e + 8..e + 16].try_into().ok()?);
                    (m, d)
                } else {
                    let d = u32::from_be_bytes(buf[e..e + 4].try_into().ok()?) as u64;
                    let m = i32::from_be_bytes(buf[e + 4..e + 8].try_into().ok()?) as i64;
                    (m, d)
                });
            }
        }
        break;
    }

    out.smpb = find_smpb(&buf);
    Some(out)
}

/// iTunSMPB lives in moov/udta/meta/ilst as a `----` freeform atom.
fn find_smpb(buf: &[u8]) -> Option<ITunSmpb> {
    let udta = child(buf, (0, buf.len()), b"udta")?;
    let meta = child(buf, udta, b"meta")?;
    // `meta` is a full box: 4 bytes of version and flags before its children.
    let ilst = child(buf, (meta.0 + 4, meta.1), b"ilst")?;
    for (kind, a, b) in atoms(buf, ilst.0, ilst.1) {
        if &kind != b"----" {
            continue;
        }
        let name = child(buf, (a, b), b"name")?;
        if &buf[name.0 + 4..name.1] != b"iTunSMPB" {
            continue;
        }
        let data = child(buf, (a, b), b"data")?;
        return ITunSmpb::parse(std::str::from_utf8(&buf[data.0 + 8..data.1]).ok()?);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    fn full(version: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![version, 0, 0, 0];
        v.extend_from_slice(body);
        v
    }

    fn mvhd(timescale: u32) -> Vec<u8> {
        let mut b = vec![0u8; 8];
        b.extend_from_slice(&timescale.to_be_bytes());
        b.extend_from_slice(&[0u8; 8]);
        full(0, &b)
    }

    fn trak(track_timescale: u32, edit: Option<(u32, i32)>) -> Vec<u8> {
        let mut mdhd = vec![0u8; 8];
        mdhd.extend_from_slice(&track_timescale.to_be_bytes());
        mdhd.extend_from_slice(&[0u8; 8]);
        let mut hdlr = b"mhlr".to_vec();
        hdlr.extend_from_slice(b"soun");
        hdlr.extend_from_slice(&[0u8; 4]);
        let mut mdia = atom(b"mdhd", &full(0, &mdhd));
        mdia.extend(atom(b"hdlr", &full(0, &hdlr)));
        let mut trak_body = atom(b"mdia", &mdia);
        if let Some((duration, media_time)) = edit {
            let mut elst = 1u32.to_be_bytes().to_vec();
            elst.extend_from_slice(&duration.to_be_bytes());
            elst.extend_from_slice(&media_time.to_be_bytes());
            elst.extend_from_slice(&[0u8; 4]);
            trak_body.extend(atom(b"edts", &atom(b"elst", &full(0, &elst))));
        }
        atom(b"trak", &trak_body)
    }

    fn smpb_ilst(value: &str) -> Vec<u8> {
        let mut name = vec![0u8; 4];
        name.extend_from_slice(b"iTunSMPB");
        let mut data = vec![0u8; 8];
        data.extend_from_slice(value.as_bytes());
        let mut body = atom(b"mean", b"com.apple.iTunes");
        body.extend(atom(b"name", &name));
        body.extend(atom(b"data", &data));
        let ilst = atom(b"ilst", &atom(b"----", &body));
        let meta = atom(b"meta", &full(0, &ilst));
        atom(b"udta", &meta)
    }

    fn write_file(moov_body: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&atom(b"ftyp", b"M4A \0\0\0\0isomiso2")).unwrap();
        f.write_all(&atom(b"moov", moov_body)).unwrap();
        f
    }

    #[test]
    fn reads_edit_list_trim_with_movie_timescale() {
        // 1024-frame priming, 44100 frames at a track timescale of 44100, movie timescale 1000.
        let mut moov = atom(b"mvhd", &mvhd(1000));
        moov.extend(trak(44100, Some((1000, 1024))));
        let f = write_file(&moov);
        let g = read(f.path()).unwrap();
        assert_eq!(g.trim(), Some((1024, Some(44100))));
    }

    #[test]
    fn itunsmpb_wins_over_edit_list() {
        let mut moov = atom(b"mvhd", &mvhd(1000));
        moov.extend(trak(44100, Some((1000, 1024))));
        moov.extend(smpb_ilst(" 00000000 00000840 000002C0 0000000000C2D500 00000000"));
        let f = write_file(&moov);
        let g = read(f.path()).unwrap();
        assert_eq!(g.trim(), Some((0x840, Some(0xC2D500))));
    }

    #[test]
    fn no_gapless_data_means_no_trim() {
        let mut moov = atom(b"mvhd", &mvhd(1000));
        moov.extend(trak(44100, None));
        let f = write_file(&moov);
        assert_eq!(read(f.path()).unwrap().trim(), None);
    }
}
