//! Agent-run 结束提示音 —— PF 原创「暖木琴」双音（A4→E5）。
//!
//! 刻意区别于 pi-web 的 C5+E5 纯正弦 chime（`hooks/useAudio.ts`）：这里音区更低、
//! 音程是纯五度（440 → 659.25 Hz），音色用加法合成模拟木琴——基频之外叠一个
//! 四次泛音（×4，横梁振动的非谐近似），泛音衰减远快于基频（0.12 s vs 0.4 s），
//! 听感厚实温和、不打扰。两音相隔 0.16 s 触发，基频包络 5 ms 线性淡入后指数
//! 衰减到 0.001。Rust 侧合成 16-bit PCM WAV，Windows 用
//! `PlaySoundW(SND_MEMORY)` 播放，不依赖任何音频资源文件。
//!
//! 候选试听稿见 `tmp/sound/gen.js`（A 钟声 / B 玻璃琶音 / C 本实现，用户选 C）。

/// 采样率（Hz）。Web Audio 默认 44100，沿用之。
const SAMPLE_RATE: u32 = 44_100;
/// 两个音的频率（Hz）：A4 与 E5（纯五度）。
const TONE_FREQS: [f64; 2] = [440.0, 659.25];
/// 第二个音相对第一个音的延迟（秒）。
const TONE_OFFSET: f64 = 0.16;
/// 加法合成的泛音倍数：基频 + 四次泛音（木琴音色近似）。
const PARTIALS: [f64; 2] = [1.0, 4.0];
/// 各泛音峰值增益。
const PARTIAL_GAINS: [f64; 2] = [0.22, 0.077];
/// 各泛音从峰值指数衰减到 [`TAIL_GAIN`] 的时长（秒）——泛音衰减更快才有
/// 木琴的「木」感，与基频共用包络会像纯弦乐。
const PARTIAL_DECAYS: [f64; 2] = [0.4, 0.12];
/// 线性淡入时长（秒）。
const ATTACK: f64 = 0.005;
/// 衰减终点增益（对齐 Web Audio 惯例的 0.001，非 0，避免指数曲线除零）。
const TAIL_GAIN: f64 = 0.001;

/// 单个泛音的包络：0 → 1.0 线性淡入，再指数衰减到 [`TAIL_GAIN`]，`len` 之后硬切 0。
fn envelope(t: f64, len: f64) -> f64 {
    if t < 0.0 {
        return 0.0;
    }
    if t < ATTACK {
        return t / ATTACK;
    }
    if t < len {
        // exponentialRamp 口径：t=ATTACK 时为 1.0，t=len 时为 TAIL_GAIN。
        let decay = (len - ATTACK).max(1e-9);
        return TAIL_GAIN.powf((t - ATTACK) / decay);
    }
    0.0
}

/// 总时长（秒）：第二个音开始时刻 + 最长泛音衰减时长。
fn total_duration() -> f64 {
    TONE_OFFSET + PARTIAL_DECAYS.iter().copied().fold(0.0_f64, f64::max)
}

/// 合成暖木琴双音提示音，返回 16-bit PCM mono WAV 字节。
pub fn done_chime_wav() -> Vec<u8> {
    let total = total_duration();
    let frames = (SAMPLE_RATE as f64 * total).ceil() as usize;
    let data_len = frames * 2; // mono / 16-bit

    let mut samples = Vec::with_capacity(frames);
    for n in 0..frames {
        let t = n as f64 / SAMPLE_RATE as f64;
        let mut v = 0.0;
        for (i, &freq) in TONE_FREQS.iter().enumerate() {
            let local = t - i as f64 * TONE_OFFSET;
            if local < 0.0 {
                continue;
            }
            for (p, &mult) in PARTIALS.iter().enumerate() {
                v += PARTIAL_GAINS[p]
                    * envelope(local, PARTIAL_DECAYS[p])
                    * (2.0 * std::f64::consts::PI * freq * mult * local).sin();
            }
        }
        let pcm = (v.clamp(-1.0, 1.0) * i16::MAX as f64).round() as i16;
        samples.extend_from_slice(&pcm.to_le_bytes());
    }

    let mut wav = Vec::with_capacity(44 + data_len);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_len as u32).to_le_bytes());
    wav.extend_from_slice(&samples);
    wav
}

/// 播放 agent-run 结束提示音（暖木琴双音）；非 Windows 平台为 no-op。
pub fn play_notify_sound() {
    #[cfg(windows)]
    {
        use std::sync::OnceLock;
        // SND_MEMORY 下 pszSound 被解释为指向内存中 WAV 数据的指针；
        // SND_ASYNC 要求这块内存在播放结束前一直有效，所以只合成一次并常驻。
        const SND_ASYNC: u32 = 0x0001;
        const SND_NODEFAULT: u32 = 0x0002;
        const SND_MEMORY: u32 = 0x0004;
        static WAV: OnceLock<&'static [u8]> = OnceLock::new();
        let bytes = WAV.get_or_init(|| Box::leak(done_chime_wav().into_boxed_slice()));
        unsafe {
            #[link(name = "winmm")]
            unsafe extern "system" {
                fn PlaySoundW(
                    psz_sound: *const u16,
                    hmod: *mut core::ffi::c_void,
                    fdw_sound: u32,
                ) -> i32;
            }
            PlaySoundW(
                bytes.as_ptr() as *const u16,
                std::ptr::null_mut(),
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
    #[cfg(not(windows))]
    {
        use std::sync::OnceLock;
        // macOS/Linux 没有 winmm；走单声道 WAV 落到临时文件 + 系统播放器
        // （macOS `afplay`，其余 `aplay`）。临时文件只合成一次并常驻，播放
        // 进程 detached — 提示音绝不能阻塞 UI 线程。
        static WAV_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
        let path = WAV_PATH.get_or_init(|| {
            let dir = std::env::temp_dir();
            let p = dir.join(format!("pi-flash-chime-{}.wav", std::process::id()));
            std::fs::write(&p, done_chime_wav()).ok()?;
            Some(p)
        });
        let Some(path) = path.as_ref() else {
            return;
        };
        #[cfg(target_os = "macos")]
        let player = "afplay";
        #[cfg(not(target_os = "macos"))]
        let player = "aplay";
        let _ = std::process::Command::new(player)
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_valid() {
        let w = done_chime_wav();
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        assert_eq!(&w[12..16], b"fmt ");
        assert_eq!(&w[36..40], b"data");
        // fmt: PCM(1) / mono(1) / 44100 / byte rate / block align / 16 bit
        assert_eq!(u16::from_le_bytes([w[20], w[21]]), 1);
        assert_eq!(u16::from_le_bytes([w[22], w[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([w[24], w[25], w[26], w[27]]),
            SAMPLE_RATE
        );
        assert_eq!(u16::from_le_bytes([w[34], w[35]]), 16);
        let data_len = u32::from_le_bytes([w[40], w[41], w[42], w[43]]) as usize;
        assert_eq!(w.len(), 44 + data_len);
        assert!(w.len() > 44);
    }

    #[test]
    fn duration_is_0_56_seconds() {
        // 0.16s 音间隔 + 0.4s 最长泛音衰减 = 0.56s
        assert!((total_duration() - 0.56).abs() < 1e-9);
        let w = done_chime_wav();
        let frames = (w.len() - 44) / 2;
        let secs = frames as f64 / SAMPLE_RATE as f64;
        assert!((secs - 0.56).abs() < 0.001, "got {secs}");
    }

    #[test]
    fn envelope_fades_and_stops() {
        let len = PARTIAL_DECAYS[0];
        assert_eq!(envelope(0.0, len), 0.0);
        assert!((envelope(ATTACK, len) - 1.0).abs() < 1e-12);
        // 指数段在 len 处收敛到 TAIL_GAIN，之后硬切到 0
        assert!((envelope(len - 1e-6, len) - TAIL_GAIN).abs() < 1e-6);
        assert_eq!(envelope(len, len), 0.0);
        assert_eq!(envelope(len + 0.1, len), 0.0);
        // 线性淡入中点
        assert!((envelope(ATTACK / 2.0, len) - 0.5).abs() < 1e-9);
        // 指数段单调下降
        let mut prev = f64::INFINITY;
        let mut t = ATTACK;
        while t < len {
            let e = envelope(t, len);
            assert!(e < prev, "envelope not decreasing at {t}");
            prev = e;
            t += 0.01;
        }
    }

    #[test]
    fn partials_decay_independently() {
        // 四次泛音 0.12s 已静音，同刻基频仍在响——「木」感的来源
        assert_eq!(envelope(0.13, PARTIAL_DECAYS[1]), 0.0);
        assert!(envelope(0.13, PARTIAL_DECAYS[0]) > 0.01);
    }

    #[test]
    fn contains_both_notes() {
        // 第二音（E5）在第一音（A4）之后启动，0.17s 附近不应静音
        let w = done_chime_wav();
        let pcm = |i: usize| -> f64 {
            let o = 44 + i * 2;
            i16::from_le_bytes([w[o], w[o + 1]]) as f64 / i16::MAX as f64
        };
        let at = |secs: f64| -> f64 {
            let n = (secs * SAMPLE_RATE as f64).round() as usize;
            (0..16).map(|k| pcm(n + k).abs()).fold(0.0, f64::max)
        };
        assert!(at(0.01) > 0.01, "first tone missing");
        assert!(at(0.17) > 0.01, "second tone missing");
    }
}
