#!/usr/bin/env python3
"""Align and compare two raw interleaved PCM streams.

Use this to compare a WASAPI loopback capture with the PCM that was sent to
the player. Requires Python 3 and NumPy (`python -m pip install numpy`).

Example:

    python scripts/compare_loopback.py expected.pcm captured.pcm \
        --format s16le --channels 2 --rate 48000 --seconds 5

It prints the detected lead-in and the exact-match ratio. Exit status 0 means
the streams matched within the configured tolerance; 1 means they did not.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np

DTYPES = {
    "s16le": np.dtype("<i2"),
    "s32le": np.dtype("<i4"),
    "f32le": np.dtype("<f4"),
}


def read_samples(path: Path, dtype: np.dtype) -> np.ndarray:
    return np.fromfile(path, dtype=dtype)


def find_offset(
    expected: np.ndarray,
    captured: np.ndarray,
    channels: int,
    probe_samples: int,
    max_offset_samples: int,
) -> int:
    """Find the sample offset of the reference inside the capture."""
    probe = expected[:probe_samples].astype(np.float64)
    probe -= probe.mean()
    search = captured[: max_offset_samples + probe_samples].astype(np.float64)
    # np.correlate is a direct sum; FFT keeps multi-second searches practical.
    size = 1 << (len(search) + len(probe) - 1).bit_length()
    correlations = np.fft.irfft(
        np.fft.rfft(search, size) * np.conj(np.fft.rfft(probe, size)), size
    )[: len(search) - len(probe) + 1]
    if correlations.size == 0:
        raise ValueError("capture is shorter than the alignment probe")
    offset = int(np.argmax(correlations))
    if correlations[offset] <= 0.0:
        raise ValueError(
            "no matching lead-in found; check --format, --channels, --rate and "
            "--max-offset-seconds, or supply --offset-frames"
        )
    if offset % channels:
        raise ValueError(
            f"detected lead-in {offset} is not frame-aligned; use --offset-frames"
        )
    return offset // channels


def compare(
    expected: np.ndarray,
    captured: np.ndarray,
    offset_frames: int,
    channels: int,
    frames: int,
    tolerance: float,
) -> tuple[int, int, float, float]:
    start = offset_frames * channels
    count = min(frames * channels, len(expected) - start, len(captured) - start)
    if count <= 0:
        raise ValueError("no overlapping frames to compare")

    reference = expected[:count].astype(np.float64)
    actual = captured[start : start + count].astype(np.float64)
    difference = np.abs(actual - reference)
    matches = int(np.count_nonzero(difference <= tolerance))
    return count // channels, matches, float(difference.max()), float(
        np.sqrt(np.mean(difference * difference))
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("expected", type=Path, help="reference raw PCM")
    parser.add_argument("captured", type=Path, help="loopback capture as raw PCM")
    parser.add_argument(
        "--format", choices=DTYPES, default="s16le", help="raw sample format (default: s16le)"
    )
    parser.add_argument("--channels", type=int, default=2, help="interleaved channels (default: 2)")
    parser.add_argument("--rate", type=int, default=48000, help="sample rate (default: 48000)")
    parser.add_argument("--seconds", type=float, default=5.0, help="frames to compare")
    parser.add_argument(
        "--max-offset-seconds",
        type=float,
        default=2.0,
        help="lead-in search window (default: 2)",
    )
    parser.add_argument(
        "--offset-frames",
        type=int,
        help="skip auto-alignment and use this exact lead-in",
    )
    parser.add_argument(
        "--tolerance",
        type=float,
        default=0.0,
        help="allowed absolute difference per sample in raw units (default: 0)",
    )
    parser.add_argument(
        "--min-match",
        type=float,
        default=1.0,
        help="required share of samples within tolerance (default: 1.0)",
    )
    args = parser.parse_args()

    if args.channels < 1 or args.rate < 1 or args.seconds <= 0:
        parser.error("--channels, --rate and --seconds must be positive")
    if not 0.0 <= args.min_match <= 1.0:
        parser.error("--min-match must be between 0 and 1")

    dtype = DTYPES[args.format]
    expected = read_samples(args.expected, dtype)
    captured = read_samples(args.captured, dtype)
    probe_samples = min(
        int(0.25 * args.rate) * args.channels, len(expected) // 4, len(captured) // 4
    )
    if probe_samples < 512 * args.channels:
        parser.error("the reference needs at least 512 frames for alignment")

    try:
        offset = (
            args.offset_frames
            if args.offset_frames is not None
            else find_offset(
                expected,
                captured,
                args.channels,
                probe_samples,
                int(args.max_offset_seconds * args.rate * args.channels),
            )
        )
        frames, matches, peak, rms = compare(
            expected,
            captured,
            offset,
            args.channels,
            int(args.seconds * args.rate),
            args.tolerance,
        )
    except ValueError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2

    ratio = matches / (frames * args.channels)
    verdict = "PASS" if ratio >= args.min_match and peak <= args.tolerance else "FAIL"
    print(f"format={args.format} channels={args.channels} rate={args.rate}")
    print(f"lead-in: {offset} frames ({offset / args.rate:.3f} s)")
    print(f"compared: {frames} frames")
    print(f"within tolerance: {matches}/{frames * args.channels} ({ratio:.6%})")
    print(f"max difference: {peak:g}; RMS difference: {rms:g}")
    print(verdict)
    return 0 if verdict == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
