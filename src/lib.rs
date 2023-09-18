use serde::Serialize;
use serde_with::skip_serializing_none;
use std::io::Cursor;

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Metadata | null")]
    pub type IMetadata;
}

#[wasm_bindgen(typescript_custom_section)]
const TS_APPEND_CONTENT: &'static str = r#"
type Format = 'MP3' | 'FLAC' | 'OPUS' | 'AAC' | 'ALAC' | 'AV1' | 'VP8' | 'VP9' | 'WAV';

type Metadata = {
    artist?: string;
    album?: string;
    title?: string;

    seconds?: number;
    format: Format;
    channels?: number;
    bitrate?: number;
    bit_depth?: number;
    sample_rate?: number;
};
"#;

#[derive(Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum Format {
    Mp3,
    Flac,
    Opus,
    Aac,
    Alac,
    Av1,
    Vp8,
    Vp9,
    Wav,
}

#[skip_serializing_none]
#[derive(Serialize)]
pub struct Metadata {
    artist: Option<String>,
    album: Option<String>,
    title: Option<String>,

    seconds: Option<f64>,

    format: Format,
    channels: Option<u32>,
    bitrate: Option<f64>,
    bit_depth: Option<u16>,
    sample_rate: Option<f64>,
}

impl Metadata {
    fn empty(format: Format) -> Self {
        Self {
            artist: None,
            album: None,
            title: None,
            seconds: None,
            format,
            channels: None,
            bitrate: None,
            bit_depth: None,
            sample_rate: None,
        }
    }
}

pub fn read_mp3(reader: &[u8]) -> Option<Metadata> {
    let mut metadata = Metadata::empty(Format::Mp3);

    if let Ok(res) = id3::Tag::read_from(reader) {
        use id3::TagLike;

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

    if let Ok(res) = mp3_metadata::read_from_slice(reader) {
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
    use metaflac::{Block, Tag};

    fn get_comment(data: Option<&Vec<String>>) -> Option<&String> {
        data?.first()
    }

    let tag = Tag::read_from(&mut { reader }).ok()?;

    let mut metadata = Metadata::empty(Format::Flac);

    for block in tag.blocks() {
        if let Block::StreamInfo(stream_info) = block {
            metadata.seconds =
                Some(stream_info.total_samples as f64 / f64::from(stream_info.sample_rate));
            metadata.channels = Some(stream_info.num_channels.into());
        } else if let Block::VorbisComment(comment) = block {
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
    use ogg_metadata::{read_format, AudioMetadata, OggFormat};

    fn format_metadata<T: AudioMetadata>(metadata: &T) -> Metadata {
        Metadata {
            channels: Some(metadata.get_output_channel_count().into()),
            seconds: metadata
                .get_duration()
                .map(|duration| duration.as_secs_f64()),
            ..Metadata::empty(Format::Opus)
        }
    }

    read_format(Cursor::new(reader)).ok().and_then(|formats| {
        formats.iter().find_map(|format| match format {
            OggFormat::Opus(res) => Some(format_metadata(res)),
            OggFormat::Vorbis(res) => Some(format_metadata(res)),
            _ => None,
        })
    })
}

pub fn read_mp4(reader: &[u8]) -> Option<Metadata> {
    use mp4parse::{
        read_mp4, AudioSampleEntry, CodecType, SampleDescriptionBox, SampleEntry, Track, TrackType,
    };

    let ctx = read_mp4(&mut { reader }).ok()?;

    ctx.tracks
        .iter()
        .filter(|Track { track_type, .. }| track_type == &TrackType::Audio)
        .filter_map(|track @ Track { stsd, .. }| stsd.as_ref().map(|stsd| (track, stsd)))
        .find_map(|(track, SampleDescriptionBox { descriptions, .. })| {
            descriptions.iter().find_map(|entry| match entry {
                SampleEntry::Audio(entry) => Some((track, entry)),
                _ => None,
            })
        })
        .and_then(
            |(
                track,
                &AudioSampleEntry {
                    codec_type,
                    channelcount,
                    samplesize,
                    samplerate,
                    ..
                },
            )| {
                let format = match codec_type {
                    CodecType::MP3 => Format::Mp3,
                    CodecType::AAC => Format::Aac,
                    CodecType::ALAC => Format::Alac,
                    CodecType::AV1 => Format::Av1,
                    CodecType::Opus => Format::Opus,
                    CodecType::FLAC => Format::Flac,
                    CodecType::VP8 => Format::Vp8,
                    CodecType::VP9 => Format::Vp9,
                    _ => return None,
                };

                Some(Metadata {
                    channels: Some(channelcount),
                    sample_rate: Some(samplerate),
                    bit_depth: Some(samplesize),
                    seconds: track.duration.and_then(|duration| {
                        track
                            .timescale
                            .map(|timescale| (duration.0 as f64 / timescale.0 as f64))
                    }),
                    ..Metadata::empty(format)
                })
            },
        )
}

pub fn read_wav(reader: &[u8]) -> Option<Metadata> {
    use hound::{WavReader, WavSpec};

    let reader = WavReader::new(reader).ok()?;

    let WavSpec {
        channels,
        sample_rate,
        bits_per_sample,
        ..
    } = reader.spec();

    Some(Metadata {
        seconds: Some(f64::from(reader.duration()) / f64::from(sample_rate)),
        sample_rate: Some(sample_rate.into()),
        bit_depth: Some(bits_per_sample),
        channels: Some(channels.into()),
        bitrate: Some(
            f64::from(sample_rate) * f64::from(channels) * f64::from(bits_per_sample) / 1_024_f64,
        ),
        ..Metadata::empty(Format::Wav)
    })
}

#[wasm_bindgen]
pub fn fazer(data: Vec<u8>) -> Result<IMetadata, JsError> {
    let metadata = read_mp4(&data)
        .or_else(|| read_ogg(&data))
        .or_else(|| read_flac(&data))
        .or_else(|| read_wav(&data))
        .or_else(|| read_mp3(&data));

    Ok(serde_wasm_bindgen::to_value(&metadata)?.unchecked_into())
}
