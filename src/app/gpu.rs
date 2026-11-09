//! Which graphics API the game draws with, what a crashed start of them leaves behind, and
//! the programs that hook into the drawing (and crash there).

#[allow(unused_imports)]
use crate::*;

/// The graphics APIs to draw with, from the `graphics_api` setting. "auto" is DX12 on Windows
/// (whatever the card: Vulkan drivers there have crashed on start, Intel's igvk64.dll on a UHD
/// 630 among them) unless no card has it, and wgpu's choice (Vulkan first) elsewhere. After a
/// Windows start that never got going (`gpu_start`) the setting itself becomes "vulkan".
pub fn graphics_backends(api: &str) -> wgpu::Backends {
    match api {
        "vulkan" => wgpu::Backends::VULKAN,
        "dx12" => wgpu::Backends::DX12,
        "gl" => wgpu::Backends::GL,
        "metal" => wgpu::Backends::METAL,
        _ => auto_backends(),
    }
}

#[cfg(windows)]
pub(crate) fn auto_backends() -> wgpu::Backends {
    // DXGI lists the cards without touching their Vulkan drivers.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let cards = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12))
        .iter()
        .filter(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)
        .count();
    if cards > 0 {
        return wgpu::Backends::DX12;
    }
    eprintln!("graphics: no DX12 graphics card: wgpu's choice");
    wgpu::Backends::all()
}

#[cfg(not(windows))]
pub(crate) fn auto_backends() -> wgpu::Backends {
    wgpu::Backends::all()
}

/// A start of the graphics that crashes (in the driver, where nothing of ours runs) leaves its
/// mark behind: `gpu-start` in the data folder, written before the graphics start and removed
/// once frames have been going for a few seconds. The next start finds it and takes the other
/// way (`auto_backends`).
pub(crate) mod gpu_start {
    const FILE: &str = "gpu-start";
    /// Frames drawn before the start counts as having worked.
    pub const FRAMES: u32 = 180;

    fn path() -> Option<std::path::PathBuf> {
        crate::config::data_dir().map(|d| d.join(FILE))
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn failed_last_time() -> bool {
        path().is_some_and(|p| p.exists())
    }

    pub fn starting() {
        if let Some(p) = path() {
            let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
            let _ = std::fs::write(
                p,
                "the graphics are starting; this goes once they're going\n",
            );
        }
    }

    pub fn going() {
        if let Some(p) = path() {
            let _ = std::fs::remove_file(p);
        }
    }

    /// The mark of a crash in an overlay or capture hook while drawing with Vulkan on "auto"
    /// (`native_crashes`): the next start takes DX12.
    const HOOK_FILE: &str = "vulkan-hook-crash";

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn hook_crashed() {
        if let Some(d) = crate::config::data_dir() {
            let _ = std::fs::write(
                d.join(HOOK_FILE),
                "a hook crashed the game drawing with Vulkan; the next start takes DX12\n",
            );
        }
    }

    /// Whether the last run ended that way (and forgets it).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn hook_crashed_last_time() -> bool {
        crate::config::data_dir().is_some_and(|d| std::fs::remove_file(d.join(HOOK_FILE)).is_ok())
    }
}

/// Programs that put their own code into the game's drawing (game capture, overlays) and
/// sometimes crash there, as (module, what it is). OBS's game capture crashed a game drawing
/// with Vulkan: graphics-hook64.dll, hooked into the present, made a D3D11 device for its shared
/// texture and dxcore.dll read 0xffffffffffffffff.
pub(crate) const DRAWING_HOOKS: [(&str, &str); 4] = [
    ("graphics-hook64.dll", "OBS's game capture"),
    ("gameoverlayrenderer64.dll", "the Steam overlay"),
    ("discordhook64.dll", "the Discord overlay"),
    ("rtsshooks64.dll", "RivaTuner (RTSS)"),
];

/// The first of `DRAWING_HOOKS` in a crash's `frames:` line.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn drawing_hook(frames: &str) -> Option<(&'static str, &'static str)> {
    let frames = frames.to_ascii_lowercase();
    frames.split_whitespace().find_map(|f| {
        DRAWING_HOOKS
            .iter()
            .find(|(dll, _)| f.split('+').next() == Some(dll))
            .copied()
    })
}

/// What the crash window says to do about a crash in `who`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn drawing_hook_advice(who: &str, dll: &str, to_dx12: bool, vulkan: bool) -> String {
    let mut out = format!(
        "It crashed inside {who} ({dll}), which hooks into the game's drawing, not in the game's own code."
    );
    if to_dx12 {
        out.push_str(
            " The next start draws with DX12 instead of Vulkan, which usually gets along with it.",
        );
    } else if vulkan {
        out.push_str(" Picking DX12 in Settings > Video > Renderer may avoid it.");
    }
    if dll == "graphics-hook64.dll" {
        out.push_str(" If it happens again, capture the game in OBS with Window Capture instead of Game Capture.");
    } else {
        out.push_str(" If it happens again, turning that overlay off avoids it.");
    }
    out
}

/// Drawing with Vulkan because `graphics_api` is "auto": a crash in a hook then moves the next
/// start to DX12.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) static VULKAN_ON_AUTO: AtomicBool = AtomicBool::new(false);
/// Drawing with Vulkan at all (for the crash window's advice).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) static VULKAN: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
mod drawing_hook_tests {
    use super::*;

    #[test]
    fn finds_obs_game_capture_in_the_frames() {
        let frames = "frames: taiko-rs.exe+0x7b154f KERNELBASE.dll+0x10f4e3 dxcore.dll+0x26d72 d3d11.dll+0xd2f7 graphics-hook64.dll+0x9b45 taiko-rs.exe+0xa1ee83";
        assert_eq!(
            drawing_hook(frames),
            Some(("graphics-hook64.dll", "OBS's game capture"))
        );
        assert_eq!(
            drawing_hook("frames: GameOverlayRenderer64.dll+0x10 taiko-rs.exe+0x1").map(|h| h.1),
            Some("the Steam overlay")
        );
        assert_eq!(
            drawing_hook("frames: taiko-rs.exe+0x1 nvoglv64.dll+0x2"),
            None
        );
        assert_eq!(drawing_hook("frames: not-graphics-hook64.dll+0x2"), None);
    }

    #[test]
    fn advice_says_what_happens_next() {
        let a = drawing_hook_advice("OBS's game capture", "graphics-hook64.dll", true, true);
        assert!(a.contains("DX12 instead of Vulkan") && a.contains("Window Capture"));
        let b = drawing_hook_advice(
            "the Steam overlay",
            "gameoverlayrenderer64.dll",
            false,
            true,
        );
        assert!(b.contains("Settings > Video > Renderer") && b.contains("overlay off"));
        assert!(
            !drawing_hook_advice(
                "the Steam overlay",
                "gameoverlayrenderer64.dll",
                false,
                false
            )
            .contains("DX12")
        );
    }
}
