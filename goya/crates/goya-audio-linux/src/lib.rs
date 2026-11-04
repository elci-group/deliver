//! goya-audio-linux — audio device lifecycle and output contract layer.
//!
//! Contract: [`AUDIO_LINUX_CONTRACT`] = `"GY-AUDIO-LINUX/1"`.
//!
//! This crate defines the device state machine and output trait without any
//! PipeWire (or other) C linkage. [`NullOutput`] is the missing-device
//! fallback: the application remains fully operational without audio —
//! writes always succeed and the pipeline completes.

/// Versioned contract identifier for this crate's audio lifecycle surface.
pub const AUDIO_LINUX_CONTRACT: &str = "GY-AUDIO-LINUX/1";

use std::fmt;

/// Lifecycle state of an audio device.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum DeviceState {
    /// Device is present and ready for output.
    #[default]
    Available,
    /// No usable device exists.
    Unavailable,
    /// Device exists but is temporarily suspended (e.g. power management).
    Suspended,
}

impl fmt::Display for DeviceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceState::Available => write!(f, "available"),
            DeviceState::Unavailable => write!(f, "unavailable"),
            DeviceState::Suspended => write!(f, "suspended"),
        }
    }
}

/// Descriptive information about an audio output device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Human-readable device name.
    pub name: String,
    /// Output channel count (mono = 1, stereo = 2, ...).
    pub channels: u8,
    /// Whether this is the system default device.
    pub default: bool,
    /// Current lifecycle state.
    pub state: DeviceState,
}

/// Errors reported by audio output backends. Never panics the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// The device is not writable in its current state.
    NotWritable(DeviceState),
    /// The backend reported an unspecified failure; details in the payload.
    Backend(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::NotWritable(state) => write!(f, "device not writable: {state}"),
            AudioError::Backend(detail) => write!(f, "audio backend error: {detail}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Contract for writing interleaved f32 samples to an audio output.
pub trait AudioOutput {
    /// Write `samples` to the output, returning the number of samples
    /// accepted. Implementations must not panic and must degrade gracefully.
    fn write(&mut self, samples: &[f32]) -> Result<usize, AudioError>;

    /// Current lifecycle state of the underlying device.
    fn state(&self) -> DeviceState;
}

/// Missing-device fallback output.
///
/// Always accepts every sample (returns `Ok(len)`) even though its state is
/// [`DeviceState::Unavailable`]: the application stays operational without
/// audio, discarding samples at this boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NullOutput {
    /// Total samples accepted, for diagnostics.
    pub accepted: u64,
}

impl NullOutput {
    /// Create a null output.
    pub fn new() -> Self {
        Self { accepted: 0 }
    }
}

impl AudioOutput for NullOutput {
    fn write(&mut self, samples: &[f32]) -> Result<usize, AudioError> {
        self.accepted += samples.len() as u64;
        Ok(samples.len())
    }

    fn state(&self) -> DeviceState {
        DeviceState::Unavailable
    }
}

/// Device lifecycle monitor.
///
/// Tracks the last observed state and any error detail. `poll` never panics
/// and degrades gracefully: it reports the current state (defaulting to
/// [`DeviceState::Unavailable`]) and records errors instead of propagating
/// them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceMonitor {
    /// Detail of the most recent error, if any.
    pub last_error: Option<String>,
    state: DeviceState,
}

impl DeviceMonitor {
    /// New monitor starting in the `Unavailable` state (safe default: no
    /// device assumed until one is observed).
    pub fn new() -> Self {
        Self {
            last_error: None,
            state: DeviceState::Unavailable,
        }
    }

    /// Poll the device subsystem. Never panics; on internal failure it
    /// degrades to `Unavailable` and records the error.
    pub fn poll(&mut self) -> DeviceState {
        match self.probe() {
            Ok(state) => {
                self.state = state;
                self.last_error = None;
            }
            Err(err) => {
                self.last_error = Some(err.to_string());
                self.state = DeviceState::Unavailable;
            }
        }
        self.state.clone()
    }

    /// Last observed state without re-probing.
    pub fn state(&self) -> &DeviceState {
        &self.state
    }

    /// Probe hook. In this contract layer there is no real backend, so the
    /// probe always succeeds with `Unavailable`; a backend integration would
    /// override this in a concrete monitor.
    fn probe(&self) -> Result<DeviceState, AudioError> {
        Ok(DeviceState::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_output_writes_always_succeed() {
        let mut out = NullOutput::new();
        assert_eq!(out.write(&[0.1, -0.1, 0.2]), Ok(3));
        assert_eq!(out.write(&[]), Ok(0));
        assert_eq!(out.accepted, 3);
        assert_eq!(out.state(), DeviceState::Unavailable);
    }

    #[test]
    fn monitor_poll_never_panics_and_degrades() {
        let mut mon = DeviceMonitor::new();
        let state = mon.poll();
        assert_eq!(state, DeviceState::Unavailable);
        assert_eq!(mon.last_error, None);
        assert_eq!(mon.state(), &DeviceState::Unavailable);
    }

    #[test]
    fn null_output_pipeline_completes() {
        // Contract test: a full render pipeline over NullOutput still
        // completes. Sources are generated locally (deterministic decaying
        // sine) so this crate has no DSP dependency at the manifest level.
        let mut out = NullOutput::new();
        let total = 2400usize;
        let mut buf = Vec::with_capacity(64);
        let mut written = 0usize;
        for i in 0..total {
            let t = i as f32 / 48_000.0;
            let s = if t < 0.05 {
                0.5 * (-40.0 * t).exp() * (440.0 * t)
            } else {
                0.0
            };
            buf.push(s);
            if buf.len() == 64 {
                match out.write(&buf) {
                    Ok(n) => written += n,
                    Err(_) => panic!("null output must never fail writes"),
                }
                buf.clear();
            }
        }
        if !buf.is_empty() {
            assert_eq!(out.write(&buf), Ok(buf.len()));
            written += buf.len();
        }
        assert_eq!(written, total);
        assert_eq!(out.accepted, total as u64);
        assert_eq!(out.state(), DeviceState::Unavailable);
    }

    #[test]
    fn audio_error_display_is_informative() {
        let e = AudioError::NotWritable(DeviceState::Suspended);
        assert!(e.to_string().contains("suspended"));
        let e = AudioError::Backend("pw stream failed".into());
        assert!(e.to_string().contains("pw stream failed"));
    }
}
