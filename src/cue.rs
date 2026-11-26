use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Default)]
pub struct Track {
    pub number: u32,
    pub title: String,
    pub tagged_title: bool,
    pub artist: String,
    pub album: String,
    pub audio: PathBuf,
    pub start: u64,
    pub end: Option<u64>,
}

pub fn is_sheet(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cue"))
}

pub fn reference(path: &Path) -> Option<(&Path, u32)> {
    let sheet = path.parent()?;
    let number = path.file_name()?.to_str()?.parse().ok()?;
    if !(1..=99).contains(&number) {
        return None;
    }
    (is_sheet(sheet) || has_embedded(sheet)).then_some((sheet, number))
}

/// True when the audio file carries a usable embedded CUE sheet.
pub fn has_embedded(path: &Path) -> bool {
    embedded(path).is_some()
}

pub fn resolve(path: &Path) -> Result<Option<Track>, String> {
    let Some((sheet, number)) = reference(path) else {
        return Ok(None);
    };
    tracks(sheet)?
        .into_iter()
        .find(|track| track.number == number)
        .map(Some)
        .ok_or_else(|| format!("CUE track {number} is missing from {}", sheet.display()))
}

pub fn entries(sheet: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(tracks(sheet)?
        .into_iter()
        .map(|track| sheet.join(track.number.to_string()))
        .collect())
}

fn tracks(source: &Path) -> Result<Vec<Track>, String> {
    if is_sheet(source) {
        read(source)
    } else {
        embedded(source).ok_or_else(|| format!("No embedded CUE in {}", source.display()))
    }
}

/// Reads `CUESHEET` text from a FLAC Vorbis comment metadata block.
///
/// lofty maps unknown Vorbis keys into a format-specific companion tag that is
/// not exposed through the generic `Tag`, so the block is parsed directly.
fn embedded(audio: &Path) -> Option<Vec<Track>> {
    let text = flac_comments(audio)
        .into_iter()
        .filter(|(key, _)| {
            key.eq_ignore_ascii_case("CUESHEET") || key.eq_ignore_ascii_case("CUESHEET_TEXT")
        })
        .map(|(_, value)| value)
        .collect::<Vec<_>>()
        .join("\n");
    parse_inner(audio, Some(audio), &text).ok()
}

fn flac_comments(path: &Path) -> Vec<(String, String)> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() || &magic != b"fLaC" {
        return Vec::new();
    }
    loop {
        let mut header = [0u8; 4];
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let last = header[0] & 0x80 != 0;
        let kind = header[0] & 0x7f;
        let length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
        if length > 16 * 1024 * 1024 {
            break;
        }
        if kind == 4 {
            let mut block = vec![0u8; length];
            if file.read_exact(&mut block).is_err() {
                break;
            }
            return parse_vorbis_block(&block).unwrap_or_default();
        }
        if file.seek(SeekFrom::Current(length as i64)).is_err() {
            break;
        }
        if last {
            break;
        }
    }
    Vec::new()
}

fn parse_vorbis_block(block: &[u8]) -> Option<Vec<(String, String)>> {
    fn field<'a>(bytes: &'a [u8], offset: &mut usize) -> Option<&'a [u8]> {
        let length = u32::from_le_bytes(bytes.get(*offset..*offset + 4)?.try_into().ok()?);
        *offset = offset.checked_add(4)?;
        let start = *offset;
        *offset = offset.checked_add(length as usize)?;
        bytes.get(start..*offset)
    }
    let mut offset = 0;
    field(block, &mut offset)?;
    let count = u32::from_le_bytes(block.get(offset..offset + 4)?.try_into().ok()?);
    offset += 4;
    let mut pairs = Vec::new();
    for _ in 0..count.min(4096) {
        let Some(item) = field(block, &mut offset) else {
            break;
        };
        let Some(separator) = item.iter().position(|&byte| byte == b'=') else {
            continue;
        };
        pairs.push((
            String::from_utf8_lossy(&item[..separator]).into_owned(),
            String::from_utf8_lossy(&item[separator + 1..]).into_owned(),
        ));
    }
    Some(pairs)
}

pub fn duration(frames: u64) -> Duration {
    Duration::from_secs(frames / 75)
        + Duration::from_nanos(((frames % 75) * 1_000_000_000 + 37) / 75)
}

fn read(sheet: &Path) -> Result<Vec<Track>, String> {
    let file = fs::File::open(sheet).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    use std::io::Read;
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("CUE sheet exceeds 1 MiB".into());
    }
    let encoding = encoding_rs::Encoding::for_bom(&bytes)
        .map(|(encoding, _)| encoding)
        .unwrap_or_else(|| {
            if std::str::from_utf8(&bytes).is_ok() {
                encoding_rs::UTF_8
            } else {
                encoding_rs::WINDOWS_1252
            }
        });
    parse(sheet, &encoding.decode(&bytes).0)
}

fn parse(sheet: &Path, text: &str) -> Result<Vec<Track>, String> {
    parse_inner(sheet, None, text)
}

fn parse_inner(
    sheet: &Path,
    default_audio: Option<&Path>,
    text: &str,
) -> Result<Vec<Track>, String> {
    let mut title = String::new();
    let mut artist = String::new();
    let mut audio = default_audio.map(Path::to_path_buf);
    let mut seen_track = false;
    let mut current = None::<Track>;
    let mut tracks: Vec<Track> = Vec::new();
    for line in text.lines().map(str::trim) {
        let Some((command, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim();
        match command.to_ascii_uppercase().as_str() {
            "FILE" => {
                let name = if let Some(quoted) = rest.strip_prefix('"') {
                    quoted.split_once('"').map(|(name, _)| name)
                } else {
                    rest.rsplit_once(char::is_whitespace)
                        .map(|(name, _)| name.trim())
                }
                .filter(|name| !name.is_empty())
                .ok_or("Invalid CUE FILE entry")?;
                audio = Some(sheet.parent().unwrap_or_else(|| Path::new(".")).join(name));
            }
            "TRACK" => {
                seen_track = true;
                let mut tokens = rest.split_whitespace();
                let number = tokens
                    .next()
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|number| (1..=99).contains(number))
                    .ok_or("Invalid CUE track number")?;
                current = tokens
                    .next()
                    .filter(|kind| kind.eq_ignore_ascii_case("AUDIO"))
                    .map(|_| Track {
                        number,
                        ..Track::default()
                    });
            }
            "TITLE" if !seen_track => title = unquote(rest).into(),
            "PERFORMER" if !seen_track => artist = unquote(rest).into(),
            "TITLE" | "PERFORMER" => {
                if let Some(track) = &mut current {
                    if command.eq_ignore_ascii_case("TITLE") {
                        track.title = unquote(rest).into();
                    } else {
                        track.artist = unquote(rest).into();
                    }
                }
            }
            "INDEX" => {
                let mut values = rest.split_whitespace();
                if values.next() != Some("01") {
                    continue;
                }
                if let Some(mut track) = current.take() {
                    track.start = values
                        .next()
                        .and_then(timestamp)
                        .ok_or("Invalid CUE INDEX 01")?;
                    track.audio = audio.clone().ok_or("CUE INDEX precedes FILE")?;
                    track.album.clone_from(&title);
                    if track.artist.is_empty() {
                        track.artist.clone_from(&artist);
                    }
                    track.tagged_title = !track.title.trim().is_empty();
                    if !track.tagged_title {
                        track.title = format!("Track {:02}", track.number);
                    }
                    if let Some(previous) = tracks.last_mut() {
                        // A new FILE restarts its indexes at zero. Only bound a
                        // track when the next one shares its source audio.
                        if previous.audio == track.audio {
                            if track.start <= previous.start {
                                return Err("CUE indexes are out of order".into());
                            }
                            previous.end = Some(track.start);
                        }
                        if track.number <= previous.number {
                            return Err("CUE tracks are out of order".into());
                        }
                    }
                    tracks.push(track);
                }
            }
            _ => {}
        }
    }
    if tracks.is_empty() {
        return Err("No indexed audio tracks in the CUE sheet".into());
    }
    Ok(tracks)
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
}

fn timestamp(value: &str) -> Option<u64> {
    let mut values = value.split(':');
    let minutes: u64 = values.next()?.parse().ok()?;
    let seconds: u64 = values.next()?.parse().ok()?;
    let frames: u64 = values.next()?.parse().ok()?;
    if seconds >= 60 || frames >= 75 || values.next().is_some() {
        return None;
    }
    minutes
        .checked_mul(4500)?
        .checked_add(seconds * 75 + frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_file_with_exact_boundaries_and_track_metadata() -> Result<(), String> {
        let tracks = parse(
            Path::new("Disc/disc.cue"),
            "TITLE \"Album\"\nPERFORMER \"Band\"\nFILE \"whole disc.flac\" WAVE\nTRACK 01 AUDIO\nTITLE \"First\"\nINDEX 01 00:00:00\nTRACK 02 AUDIO\nTITLE \"Second\"\nINDEX 01 00:13:37",
        )?;
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].end, Some(1012));
        assert_eq!(tracks[1].start, 1012);
        assert_eq!(tracks[1].title, "Second");
        assert_eq!(tracks[1].artist, "Band");
        assert_eq!(tracks[1].album, "Album");
        Ok(())
    }

    #[test]
    fn parses_multiple_files_without_bounding_the_last_track_of_each_file() -> Result<(), String> {
        let tracks = parse(
            Path::new("Album/album.cue"),
            "TITLE \"Album\"\nFILE \"01.flac\" WAVE\nTRACK 01 AUDIO\nTITLE \"First\"\nINDEX 01 00:00:00\nFILE \"02.flac\" WAVE\nTRACK 02 AUDIO\nTITLE \"Second\"\nINDEX 01 00:00:00\nTRACK 03 AUDIO\nTITLE \"Third\"\nINDEX 01 01:00:00",
        )?;
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].audio, PathBuf::from("Album/01.flac"));
        assert_eq!(tracks[0].end, None);
        assert_eq!(tracks[1].audio, PathBuf::from("Album/02.flac"));
        assert_eq!(tracks[1].start, 0);
        assert_eq!(tracks[1].end, Some(4500));
        assert_eq!(tracks[2].audio, PathBuf::from("Album/02.flac"));
        assert_eq!(tracks[2].end, None);
        assert!(parse(
            Path::new("Album/album.cue"),
            "FILE \"a.wav\" WAVE\nTRACK 01 AUDIO\nINDEX 01 00:10:00\nTRACK 02 AUDIO\nINDEX 01 00:05:00"
        )
        .is_err());
        Ok(())
    }

    fn flac_with_cuesheet(text: &str) -> Vec<u8> {
        let mut entry = b"CUESHEET=".to_vec();
        entry.extend_from_slice(text.as_bytes());
        let mut item = Vec::new();
        item.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        item.extend_from_slice(&entry);
        let mut block = Vec::new();
        block.extend_from_slice(&4u32.to_le_bytes());
        block.extend_from_slice(b"test");
        block.extend_from_slice(&1u32.to_le_bytes());
        block.extend_from_slice(&item);
        let mut bytes = b"fLaC".to_vec();
        bytes.push(0x80 | 4);
        bytes.extend_from_slice(&(block.len() as u32).to_be_bytes()[1..]);
        bytes.extend_from_slice(&block);
        bytes
    }

    #[test]
    fn parses_embedded_cue_without_file_entry() -> Result<(), String> {
        let tracks = parse_inner(
            Path::new("Album/whole.flac"),
            Some(Path::new("Album/whole.flac")),
            "TITLE \"Album\"\nTRACK 01 AUDIO\nTITLE \"First\"\nINDEX 01 00:00:00\nTRACK 02 AUDIO\nTITLE \"Second\"\nINDEX 01 00:13:37",
        )?;
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].audio, PathBuf::from("Album/whole.flac"));
        assert_eq!(tracks[0].title, "First");
        assert_eq!(tracks[1].start, 1012);
        assert_eq!(tracks[1].title, "Second");
        Ok(())
    }

    #[test]
    fn resolves_virtual_track_from_flac_cuesheet_comment() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let audio = directory.path().join("whole.flac");
        let text = "TITLE \"Album\"\nTRACK 01 AUDIO\nTITLE \"First\"\nINDEX 01 00:00:00\nTRACK 02 AUDIO\nTITLE \"Second\"\nINDEX 01 00:13:37";
        let bytes = flac_with_cuesheet(text);
        assert_eq!(&bytes[..4], b"fLaC");
        assert_eq!(bytes[4], 0x84);
        assert_eq!(
            u32::from_be_bytes([0, bytes[5], bytes[6], bytes[7]]) as usize,
            bytes.len() - 8
        );
        std::fs::write(&audio, bytes)?;
        let comments = flac_comments(&audio);
        assert_eq!(comments.len(), 1, "comments: {comments:?}");
        assert!(embedded(&audio).is_some(), "embedded CUE missing");
        let virtual_track = audio.join("2");
        let (source, number) = reference(&virtual_track).ok_or("missing CUE reference")?;
        assert_eq!(source, audio.as_path());
        assert_eq!(number, 2);
        let track = resolve(&virtual_track)?.ok_or("missing CUE track")?;
        assert_eq!(track.title, "Second");
        assert_eq!(track.audio, audio);
        assert_eq!(entries(&audio)?, vec![audio.join("1"), audio.join("2")]);
        Ok(())
    }
}
