//! System-audio loopback capture for the "Audiowave Orb" rice skin.
//!
//! WASAPI loopback on the default render endpoint (shared mode) → mono mix →
//! `rustfft` over a Hann-windowed window → 36 log-spaced magnitude bands
//! (0..1), emitted as the `audio-spectrum` Tauri event ~30×/sec. The frontend
//! orb reads those bands as its spectrum (replacing the mockup's fake
//! generator). Capture runs only while the orb skin is active (`set_active`),
//! so the Classic skin costs nothing.
//!
//! Loopback recipe (wasapi 0.23): `AUDCLNT_STREAMFLAGS_LOOPBACK` is set for the
//! (device = Render, stream = Capture, Shared) combination — i.e. take the
//! default *render* device but initialize its client in the *capture*
//! direction. What you then capture is the exact mix played to the speakers
//! (all apps), which is what we want for a "reacts to whatever's playing" ring.

#[cfg(windows)]
pub use imp::set_active;

/// No system-audio capture off Windows (the app ships Windows-only); the orb
/// skin then just renders a flat ring. Keeps the module cross-compiling.
#[cfg(not(windows))]
pub fn set_active(_app: &tauri::AppHandle, _on: bool) {}

#[cfg(windows)]
mod imp {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;

    use rustfft::{num_complex::Complex, FftPlanner};
    use tauri::{AppHandle, Emitter};
    use wasapi::{
        initialize_mta, Direction, DeviceEnumerator, SampleType, StreamMode, WaveFormat,
    };

    /// Bands emitted per frame — must match the orb engine's `spec[]` length.
    const BANDS: usize = 36;
    /// FFT window: 2048 @ 44.1 kHz ≈ 46 ms — enough low-end resolution
    /// (~21 Hz/bin) without visibly lagging the beat.
    const FFT_SIZE: usize = 2048;
    const SAMPLE_RATE: usize = 44100;
    /// Log band range. 30 Hz–16 kHz spans musical energy; below is rumble and
    /// above is mostly hiss that would just make the ring shimmer.
    const F_LO: f32 = 30.0;
    const F_HI: f32 = 16_000.0;
    /// Linear gain before the sqrt + soft 0..1 clamp. Higher = the ring reacts
    /// harder at a given loudness, so you don't have to crank the source volume
    /// to make the bars dance. GAIN 26 lowers the saturation knee to a raw
    /// magnitude of ~0.038 (vs ~0.125 at the old GAIN 8), so normal-volume music
    /// fills most of the ring; a genuinely loud mix now just pins the top, which
    /// is fine. Tunable — raise for even more sensitivity, lower to tame it.
    const GAIN: f32 = 26.0;
    /// Raw per-band magnitude below this reads as silence (→ 0), so a truly
    /// idle device shows a flat ring instead of shimmering on float/hiss noise.
    /// Raised alongside GAIN: the higher gain would otherwise amplify near-floor
    /// hiss into visible idle wobble. Real quiet music still sits well above it.
    const NOISE_FLOOR: f32 = 0.001;
    /// Emit spacing (~33 Hz). The orb's rAF runs at 60 fps and smooths each
    /// band through its own envelope follower, so a 30 Hz data feed looks fluid.
    const EMIT_EVERY_MS: u128 = 10;

    /// Set while a capture thread should keep running. `set_active` flips it;
    /// the thread checks it once per event timeout and unwinds when it clears.
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    /// Bumped once per spawned capture thread. A thread also exits when a
    /// newer generation exists: a rapid off→on toggle can respawn before the
    /// old thread has sampled the cleared flag, and without this check both
    /// threads would see ACTIVE == true and emit concurrently (doubled,
    /// jittery spectrum).
    static GEN: AtomicU64 = AtomicU64::new(0);
    /// Holds the running thread's handle so a stray double-start can't leak it.
    static THREAD: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

    /// Start or stop the loopback capture. Idempotent: starting while already
    /// running is a no-op; stopping just clears the flag and lets the thread
    /// unwind on its own within one event timeout (no blocking join on the
    /// caller, which is the UI/menu thread).
    pub fn set_active(app: &AppHandle, on: bool) {
        if on {
            if ACTIVE.swap(true, Ordering::SeqCst) {
                return; // already running
            }
            let my_gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
            let app = app.clone();
            let handle = std::thread::Builder::new()
                .name("audio-loopback".into())
                .spawn(move || {
                    if let Err(e) = capture_loop(&app, my_gen) {
                        // Missing device / format refusal: the orb just stays
                        // flat. Leave a breadcrumb on the hidden console.
                        eprintln!("clawdometer audio: loopback stopped: {e}");
                    }
                    // Whether it errored or was told to stop, leave the flag
                    // clear so a later re-activation spawns a fresh thread —
                    // unless a newer generation superseded this one and owns
                    // the flag now.
                    if GEN.load(Ordering::SeqCst) == my_gen {
                        ACTIVE.store(false, Ordering::SeqCst);
                    }
                })
                .ok();
            if handle.is_none() {
                // Spawn failed: no thread will ever clear the flag, and
                // leaving it set would turn every future start into an
                // "already running" no-op until app restart.
                ACTIVE.store(false, Ordering::SeqCst);
            }
            if let Ok(mut slot) = THREAD.lock() {
                *slot = handle;
            }
        } else {
            ACTIVE.store(false, Ordering::SeqCst);
        }
    }

    fn capture_loop(app: &AppHandle, my_gen: u64) -> Result<(), Box<dyn std::error::Error>> {
        // COM into the MTA for this thread. Harmless if already initialized.
        let _ = initialize_mta().ok();

        let enumerator = DeviceEnumerator::new()?;
        // Render device + Capture stream direction = loopback (see module docs).
        let device = enumerator.get_default_device(&Direction::Render)?;
        let mut client = device.get_iaudioclient()?;
        // 32-bit float, 44.1 kHz, stereo. autoconvert asks WASAPI to resample
        // the endpoint's native mix into exactly this, so the byte layout below
        // (8-byte interleaved-stereo f32 frames) always holds.
        let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE, 2, None);
        let (_def_time, min_time) = client.get_device_period()?;
        let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: min_time };
        client.initialize_client(&format, &Direction::Capture, &mode)?;
        let h_event = client.set_get_eventhandle()?;
        let capture = client.get_audiocaptureclient()?;
        client.start_stream()?;

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        // Hann window (sin² form) tames spectral leakage so bars don't smear.
        let hann: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let s = (std::f32::consts::PI * i as f32 / (FFT_SIZE as f32 - 1.0)).sin();
                s * s
            })
            .collect();
        let band_bins = band_bin_ranges();

        let mut raw: VecDeque<u8> = VecDeque::with_capacity(FFT_SIZE * 16);
        let mut mono: VecDeque<f32> = VecDeque::with_capacity(FFT_SIZE);
        let mut scratch = vec![Complex { re: 0.0f32, im: 0.0f32 }; FFT_SIZE];
        let mut last_emit = Instant::now();

        while ACTIVE.load(Ordering::SeqCst) && GEN.load(Ordering::SeqCst) == my_gen {
            // Pull whatever WASAPI has buffered. On silence this may add nothing.
            let _ = capture.read_from_device_to_deque(&mut raw);
            // Interleaved stereo f32, little-endian: 8 bytes per frame. Mix to
            // mono and keep a sliding window of the latest FFT_SIZE samples.
            while raw.len() >= 8 {
                let mut b = [0u8; 8];
                for slot in &mut b {
                    *slot = raw.pop_front().unwrap();
                }
                let l = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let r = f32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                if mono.len() == FFT_SIZE {
                    mono.pop_front();
                }
                mono.push_back((l + r) * 0.5);
            }

            if mono.len() == FFT_SIZE && last_emit.elapsed().as_millis() >= EMIT_EVERY_MS {
                for (i, s) in mono.iter().enumerate() {
                    scratch[i] = Complex { re: *s * hann[i], im: 0.0 };
                }
                fft.process(&mut scratch);
                let bands = fold_to_bands(&scratch, &band_bins);
                let _ = app.emit("audio-spectrum", bands);
                last_emit = Instant::now();
            }

            // Block until the next buffer is ready (or 100 ms), so CPU stays
            // near zero between callbacks and a stop request takes effect within
            // one timeout.
            let _ = h_event.wait_for_event(100);
        }

        let _ = client.stop_stream();
        Ok(())
    }

    /// Precompute the `[start, end)` FFT bin index for each of the 36 log bands.
    fn band_bin_ranges() -> Vec<(usize, usize)> {
        let nyquist = FFT_SIZE / 2;
        let ratio = (F_HI / F_LO).powf(1.0 / BANDS as f32);
        let bin_of = |f: f32| ((f / SAMPLE_RATE as f32) * FFT_SIZE as f32).round() as usize;
        (0..BANDS)
            .map(|b| {
                let lo = F_LO * ratio.powi(b as i32);
                let hi = F_LO * ratio.powi(b as i32 + 1);
                let s = bin_of(lo).clamp(1, nyquist - 1);
                let e = bin_of(hi).clamp(s + 1, nyquist);
                (s, e)
            })
            .collect()
    }

    /// Peak magnitude per band, floor-gated, gained and soft-clamped to 0..1.
    /// PEAK, not average: averaging over a band's bins flattened the response
    /// (a strong tone in one bin got diluted by its quiet neighbours, so loud
    /// and quiet audio produced nearly identical bands — measured). The peak
    /// bin tracks the actual signal; sqrt spreads the perceptual range.
    fn fold_to_bands(spectrum: &[Complex<f32>], bins: &[(usize, usize)]) -> Vec<f32> {
        let norm = 1.0 / FFT_SIZE as f32;
        bins.iter()
            .map(|&(s, e)| {
                let peak = spectrum[s..e].iter().map(|c| c.norm()).fold(0.0f32, f32::max) * norm;
                let v = (peak - NOISE_FLOOR).max(0.0);
                (v * GAIN).sqrt().min(1.0)
            })
            .collect()
    }
}
