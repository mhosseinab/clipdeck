//! Runtime-synthesized sounds (spec/behavior.md §12): original PCM tones,
//! no bundled assets. Played via the platform audio CLI (best effort).

use std::io::Write;
use std::process::{Command, Stdio};

const RATE: u32 = 22050;

pub fn play(kind: &str) {
    let wav = match kind {
        "copy" => blip(660.0, 0.10),
        _ => noise(0.06),
    };
    let dir = std::env::temp_dir();
    let path = dir.join(format!("clipdeck-sound-{kind}.wav"));
    if std::fs::write(&path, wav).is_err() {
        return;
    }
    let p = path.to_string_lossy().into_owned();
    for (cmd, args) in [
        ("paplay", vec![p.clone()]),
        ("pw-play", vec![p.clone()]),
        ("aplay", vec!["-q".into(), p.clone()]),
    ] {
        if Command::new(cmd)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return;
        }
    }
}

fn wav_header(pcm_bytes: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + pcm_bytes).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes()); // PCM
    h.extend_from_slice(&1u16.to_le_bytes()); // mono
    h.extend_from_slice(&RATE.to_le_bytes());
    h.extend_from_slice(&(RATE * 2).to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&pcm_bytes.to_le_bytes());
    h
}

/// Two-note sine blip (copy feedback).
fn blip(freq: f32, secs: f32) -> Vec<u8> {
    let n = (RATE as f32 * secs) as usize;
    let mut pcm = Vec::with_capacity(n * 2);
    let tones = [(freq, 0.55), (freq * 1.5, 0.45)];
    let per = n / tones.len();
    for (f, amp) in tones {
        for i in 0..per {
            let t = i as f32 / RATE as f32;
            let env = 1.0 - (i as f32 / per as f32);
            let s = (2.0 * std::f32::consts::PI * f * t).sin() * amp * env * 0.25;
            pcm.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
        }
    }
    let bytes = pcm.len() as u32;
    let mut out = wav_header(bytes);
    out.extend_from_slice(&pcm);
    out
}

/// Short decaying noise burst (paste feedback).
fn noise(secs: f32) -> Vec<u8> {
    let n = (RATE as f32 * secs) as usize;
    let mut pcm = Vec::with_capacity(n * 2);
    let mut seed: u32 = 0x1234_5678;
    for i in 0..n {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let r = ((seed >> 16) as f32 / u16::MAX as f32) * 2.0 - 1.0;
        let env = 1.0 - (i as f32 / n as f32);
        pcm.extend_from_slice(&((r * env * 0.2 * i16::MAX as f32) as i16).to_le_bytes());
    }
    let bytes = pcm.len() as u32;
    let mut out = wav_header(bytes);
    out.extend_from_slice(&pcm);
    let _ = Write::flush(&mut out);
    out
}
