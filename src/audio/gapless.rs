use std::{path::PathBuf, time::Duration};
use symphonia::core::{audio::SampleBuffer, conv::ConvertibleSample};

use super::{
    Event,
    decoder::{Source, SourceFormat},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Next {
    pub path: PathBuf,
    pub target: NextTarget,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextTarget {
    Playlist(usize),
    Queue(u64, crate::Track),
    Repeat(crate::Track),
}

pub struct Transition {
    pub next: Next,
    pub source: SourceFormat,
    pub duration: Option<Duration>,
    pub listened_at_boundary: Duration,
}

pub(super) struct Handoff {
    pub next: Next,
    pub previous: Source,
    pub boundary: u64,
}

pub(super) fn compatible(current: &SourceFormat, next: &SourceFormat) -> bool {
    current.rate == next.rate && current.mask == next.mask && current.kind == next.kind
}

pub(super) fn next_packet<'a, T: ConvertibleSample>(
    source: &mut Source,
    scratch: &'a mut Option<SampleBuffer<T>>,
    report: &impl Fn(Event),
) -> Option<&'a [T]> {
    match source.next(scratch) {
        Ok(samples) => samples,
        Err(error) => {
            report(Event::Warning(format!(
                "Skipped unreadable audio at the end of the track: {error}"
            )));
            None
        }
    }
}
