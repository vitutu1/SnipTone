use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeySpec {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Win32 Virtual-Key code (e.g. 'R' as u32 = 0x52, VK_F9 = 0x78)
    pub vk: u32,
}

impl HotkeySpec {
    pub const fn new(ctrl: bool, shift: bool, alt: bool, vk: u32) -> Self {
        Self {
            ctrl,
            shift,
            alt,
            vk,
        }
    }

    pub fn display_label(&self) -> String {
        let mut parts = Vec::with_capacity(4);
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        let key_name = match self.vk {
            0x41..=0x5A => {
                let ch = (self.vk as u8) as char;
                return if parts.is_empty() {
                    ch.to_string()
                } else {
                    format!("{}+{}", parts.join("+"), ch)
                };
            }
            0x2C => "PrtSc",
            0x70 => "F1",
            0x71 => "F2",
            0x72 => "F3",
            0x73 => "F4",
            0x74 => "F5",
            0x75 => "F6",
            0x76 => "F7",
            0x77 => "F8",
            0x78 => "F9",
            0x79 => "F10",
            0x7A => "F11",
            0x7B => "F12",
            _ => "Key",
        };
        if parts.is_empty() {
            key_name.to_string()
        } else {
            format!("{}+{}", parts.join("+"), key_name)
        }
    }
}

pub const REGION_PRESETS: &[HotkeySpec] = &[
    HotkeySpec::new(true, true, false, b'R' as u32),
    HotkeySpec::new(false, true, true, b'R' as u32),
    HotkeySpec::new(true, false, true, b'R' as u32),
    HotkeySpec::new(false, false, false, 0x78), // F9
];

pub const FULLSCREEN_PRESETS: &[HotkeySpec] = &[
    HotkeySpec::new(true, true, false, b'H' as u32),
    HotkeySpec::new(false, true, true, b'H' as u32),
    HotkeySpec::new(true, false, true, b'H' as u32),
    HotkeySpec::new(false, false, false, 0x79), // F10
];

pub const TOGGLE_HUD_PRESETS: &[HotkeySpec] = &[
    HotkeySpec::new(true, true, false, b'G' as u32),
    HotkeySpec::new(false, true, true, b'S' as u32),
    HotkeySpec::new(true, false, true, b'S' as u32),
    HotkeySpec::new(false, false, false, 0x7A), // F11
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutConfig {
    pub region_snip: HotkeySpec,
    pub full_screen: HotkeySpec,
    pub toggle_hud: HotkeySpec,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            region_snip: REGION_PRESETS[0],
            full_screen: FULLSCREEN_PRESETS[0],
            toggle_hud: TOGGLE_HUD_PRESETS[0],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BarPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

impl BarPosition {
    pub const ALL: &'static [Self] = &[Self::Top, Self::Bottom, Self::Left, Self::Right];

    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "Top",
            Self::Bottom => "Bottom",
            Self::Left => "Left",
            Self::Right => "Right",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub auto_copy: bool,
    pub auto_save_png: bool,
    pub capture_cursor: bool,
    pub default_delay_secs: u64,
    pub bar_position: BarPosition,
    pub start_with_windows: bool,
    pub shortcuts: ShortcutConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_copy: true,
            auto_save_png: false,
            capture_cursor: false,
            default_delay_secs: 0,
            bar_position: BarPosition::Top,
            start_with_windows: false,
            shortcuts: ShortcutConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn config_path() -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("SnipTone").join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    pub fn cycle_region_shortcut(&mut self) {
        let idx = REGION_PRESETS
            .iter()
            .position(|&p| p == self.shortcuts.region_snip)
            .map(|i| (i + 1) % REGION_PRESETS.len())
            .unwrap_or(0);
        self.shortcuts.region_snip = REGION_PRESETS[idx];
        self.save();
    }

    pub fn cycle_fullscreen_shortcut(&mut self) {
        let idx = FULLSCREEN_PRESETS
            .iter()
            .position(|&p| p == self.shortcuts.full_screen)
            .map(|i| (i + 1) % FULLSCREEN_PRESETS.len())
            .unwrap_or(0);
        self.shortcuts.full_screen = FULLSCREEN_PRESETS[idx];
        self.save();
    }

    pub fn cycle_hud_shortcut(&mut self) {
        let idx = TOGGLE_HUD_PRESETS
            .iter()
            .position(|&p| p == self.shortcuts.toggle_hud)
            .map(|i| (i + 1) % TOGGLE_HUD_PRESETS.len())
            .unwrap_or(0);
        self.shortcuts.toggle_hud = TOGGLE_HUD_PRESETS[idx];
        self.save();
    }
}
