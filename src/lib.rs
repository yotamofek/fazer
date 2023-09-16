use ogg_metadata::AudioMetadata;
use serde::Serialize;
use std::char;
use std::io::{BufReader, Cursor};
use wasm_bindgen::prelude::*;

#[derive(Serialize, Default)]
pub struct Metadata {
    artist: Option<String>,
    album: Option<String>,
    title: Option<String>,

    seconds: Option<f64>,

    format: String,
    channels: Option<u32>,
    bitrate: Option<f64>,
    bit_depth: Option<u16>,
    sample_rate: Option<f64>,
}

pub fn read_mp3(reader: &[u8]) -> Option<Metadata> {
    let mut metadata = Metadata {
        format: String::from("MP3"),
        ..Default::default()
    };

    if let Ok(res) = id3::Tag::read_from(&reader[..]) {
        if let Some(artist) = res.artist() {
            metadata.artist = Some(String::from(artist))
        } else if let Some(artist) = res.album_artist() {
            metadata.artist = Some(String::from(artist))
        }

        if let Some(album) = res.album() {
            metadata.album = Some(String::from(album))
        }

        if let Some(title) = res.title() {
            metadata.title = Some(String::from(title))
        }

        metadata.seconds = res
            .duration()
            .map(|miliseconds| f64::from(miliseconds) / 1_000_f64)
    }

    if let Ok(res) = mp3_metadata::read_from_slice(&reader[..]) {
        if let Some(frame) = res.frames.first() {
            metadata.channels = match frame.chan_type {
                mp3_metadata::ChannelType::SingleChannel => Some(1),
                mp3_metadata::ChannelType::Unknown => None,
                _ => Some(2),
            };

            metadata.bitrate = Some(frame.bitrate.into());
            metadata.sample_rate = Some(frame.sampling_freq.into());
        };

        for tag in res.optional_info {
            if metadata.title.is_none() && tag.title.is_some() {
                metadata.title = Some(tag.title.unwrap().clone())
            }

            if metadata.artist.is_none() && !tag.original_artists.is_empty() {
                metadata.artist = Some(tag.original_artists.join(", "))
            }
        }

        if let Some(ref tag) = res.tag {
            if metadata.artist.is_none() && !tag.artist.is_empty() {
                metadata.artist = Some(String::from(tag.artist.trim_end_matches('\x00')))
            }

            if metadata.album.is_none() && !tag.album.is_empty() {
                metadata.album = Some(String::from(tag.album.trim_end_matches('\x00')))
            }

            if metadata.title.is_none() && !tag.title.is_empty() {
                metadata.title = Some(String::from(tag.title.trim_end_matches('\x00')))
            }
        }

        metadata.seconds = Some(res.duration.as_secs_f64());
    }

    Some(metadata)
}

pub fn read_flac(reader: &[u8]) -> Option<Metadata> {
    fn get_comment(data: Option<&Vec<String>>) -> Option<&String> {
        data.map(|vec| vec.first()).unwrap_or_default()
    }

    let tag = match metaflac::Tag::read_from(&mut BufReader::new(&reader[..])) {
        Err(_) => {
            return None;
        }
        Ok(tag) => tag,
    };

    let mut metadata = Metadata {
        format: String::from("FLAC"),
        ..Default::default()
    };

    for block in tag.blocks() {
        if let metaflac::Block::StreamInfo(stream_info) = block {
            metadata.seconds =
                Some(stream_info.total_samples as f64 / f64::from(stream_info.sample_rate));
            metadata.channels = Some(stream_info.num_channels.into());
        } else if let metaflac::Block::VorbisComment(comment) = block {
            if let Some(artist) = get_comment(comment.artist()) {
                metadata.artist = Some(artist.clone())
            } else if let Some(artist) = get_comment(comment.album_artist()) {
                metadata.artist = Some(artist.clone())
            }

            if let Some(album) = get_comment(comment.album()) {
                metadata.album = Some(album.clone())
            }

            if let Some(title) = get_comment(comment.title()) {
                metadata.title = Some(title.clone())
            }
        }
    }

    Some(metadata)
}

pub fn read_ogg(reader: &[u8]) -> Option<Metadata> {
    let reader = BufReader::new(Cursor::new(&reader[..]));

    fn format_metadata<T: AudioMetadata>(metadata: &T) -> Metadata {
        Metadata {
            format: String::from("OPUS"),
            channels: Some(metadata.get_output_channel_count().into()),
            seconds: metadata
                .get_duration()
                .map(|duration| duration.as_secs_f64()),
            ..Default::default()
        }
    }

    if let Ok(res) = ogg_metadata::read_format(reader) {
        for format in res {
            match format {
                ogg_metadata::OggFormat::Opus(ref res) => return Some(format_metadata(res)),
                ogg_metadata::OggFormat::Vorbis(ref res) => return Some(format_metadata(res)),
                _ => {}
            }
        }
    }

    None
}

pub fn read_mp4(reader: &[u8]) -> Option<Metadata> {
    let mut ctx = mp4parse::MediaContext::new();
    let ok = mp4parse::read_mp4(&mut BufReader::new(Cursor::new(&reader[..])), &mut ctx);

    if ok.is_err() {
        return None;
    };

    for track in &ctx.tracks {
        if track.track_type != mp4parse::TrackType::Audio {
            continue;
        }

        if let Some(ref stsd) = track.stsd {
            let entry = stsd
                .descriptions
                .iter()
                .flat_map(|desc| match desc {
                    mp4parse::SampleEntry::Audio(entry) => Some(entry),
                    _ => None,
                })
                .next();

            if let Some(ref entry) = entry {
                return Some(Metadata {
                    format: String::from(match entry.codec_type {
                        mp4parse::CodecType::MP3 => "MP3",
                        mp4parse::CodecType::AAC => "AAC",
                        mp4parse::CodecType::ALAC => "ALAC",
                        mp4parse::CodecType::AV1 => "AV1",
                        mp4parse::CodecType::Opus => "OPUS",
                        mp4parse::CodecType::FLAC => "FLAC",
                        mp4parse::CodecType::VP8 => "VP8",
                        mp4parse::CodecType::VP9 => "VP9",
                        _ => return None,
                    }),

                    channels: Some(entry.channelcount),
                    sample_rate: Some(entry.samplerate),
                    bit_depth: Some(entry.samplesize),
                    seconds: track.duration.and_then(|duration| {
                        track
                            .timescale
                            .map(|timescale| (duration.0 as f64 / timescale.0 as f64))
                    }),

                    ..Default::default()
                });
            }
        }
    }

    None
}

pub fn read_wav(reader: &[u8]) -> Option<Metadata> {
    let reader = match hound::WavReader::new(&reader[..]) {
        Ok(reader) => reader,
        Err(_) => return None,
    };

    let spec = reader.spec();

    Some(Metadata {
        format: String::from("WAV"),
        seconds: Some(f64::from(reader.duration()) / f64::from(spec.sample_rate)),
        sample_rate: Some(spec.sample_rate.into()),
        bit_depth: Some(spec.bits_per_sample),
        channels: Some(spec.channels.into()),
        bitrate: Some(
            f64::from(spec.sample_rate)
                * f64::from(spec.channels)
                * f64::from(spec.bits_per_sample)
                / 1_024_f64,
        ),
        ..Default::default()
    })
}

#[wasm_bindgen(js_name = "translateHebrewGibberish")]
pub fn translate_hebrew_gibberish(data: &str) -> String {
    const LATIN_TO_HEBREW_DELTA: u32 = 'א' as u32 - 'à' as u32;

    let is_latin1 = |c: char| c >= 'à' && c <= 'ö';
    let from_latin1_to_hebrew =
        |c| unsafe { char::from_u32_unchecked(c as u32 + LATIN_TO_HEBREW_DELTA) };

    data.chars()
        .map(|c| {
            if !is_latin1(c) {
                c
            } else {
                from_latin1_to_hebrew(c)
            }
        })
        .collect()
}

#[wasm_bindgen]
pub fn fazer(data: Vec<u8>) -> JsValue {
    if let Some(metadata) = read_mp4(&data[..]) {
        JsValue::from_serde(&metadata).unwrap()
    } else if let Some(metadata) = read_ogg(&data[..]) {
        JsValue::from_serde(&metadata).unwrap()
    } else if let Some(metadata) = read_flac(&data[..]) {
        JsValue::from_serde(&metadata).unwrap()
    } else if let Some(metadata) = read_wav(&data[..]) {
        JsValue::from_serde(&metadata).unwrap()
    } else if let Some(metadata) = read_mp3(&data[..]) {
        JsValue::from_serde(&metadata).unwrap()
    } else {
        JsValue::from_serde(&()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gibberish_and_english() {
        assert_eq!(
            translate_hebrew_gibberish("ðåòí áðàé - hello"),
            "נועם בנאי - hello"
        );
    }

    #[test]
    fn test_one_word_gibberish() {
        assert_eq!(translate_hebrew_gibberish("ðåòí"), "נועם");
    }

    #[test]
    fn test_hebrew() {
        assert_eq!(translate_hebrew_gibberish("נועם"), "נועם");
    }

    #[test]
    fn test_english() {
        assert_eq!(translate_hebrew_gibberish("noam"), "noam");
    }

    #[test]
    fn test_hebrew_and_english() {
        assert_eq!(translate_hebrew_gibberish("noam בנאי"), "noam בנאי");
    }
}
