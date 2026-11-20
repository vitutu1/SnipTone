use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

pub struct IconsAssets;

impl AssetSource for IconsAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let data: Option<&'static str> = match path {
            "icons/grid.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><rect x="4" y="4" width="3.5" height="3.5" rx="0.8"/><rect x="10.25" y="4" width="3.5" height="3.5" rx="0.8"/><rect x="16.5" y="4" width="3.5" height="3.5" rx="0.8"/><rect x="4" y="10.25" width="3.5" height="3.5" rx="0.8"/><rect x="10.25" y="10.25" width="3.5" height="3.5" rx="0.8"/><rect x="16.5" y="10.25" width="3.5" height="3.5" rx="0.8"/><rect x="4" y="16.5" width="3.5" height="3.5" rx="0.8"/><rect x="10.25" y="16.5" width="3.5" height="3.5" rx="0.8"/><rect x="16.5" y="16.5" width="3.5" height="3.5" rx="0.8"/></svg>"#,
            ),
            "icons/chev.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 6 6 6-6 6"/></svg>"#,
            ),
            "icons/region.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 3H3v2M21 5V3h-2M3 19v2h2M19 21h2v-2"/><rect x="7" y="7" width="10" height="10" rx="1.5" stroke-dasharray="3 2"/></svg>"#,
            ),
            "icons/camera.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M9 3.5 7.4 5.5H4.5A2.5 2.5 0 0 0 2 8v10a2.5 2.5 0 0 0 2.5 2.5h15A2.5 2.5 0 0 0 22 18V8a2.5 2.5 0 0 0-2.5-2.5h-2.9L15 3.5H9Zm3 4.5a5 5 0 1 1 0 10 5 5 0 0 1 0-10Zm0 2a3 3 0 1 0 0 6 3 3 0 0 0 0-6Z"/></svg>"#,
            ),
            "icons/sliders.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="4" y1="20" x2="4" y2="14"/><line x1="4" y1="10" x2="4" y2="4"/><line x1="12" y1="20" x2="12" y2="12"/><line x1="12" y1="8" x2="12" y2="4"/><line x1="20" y1="20" x2="20" y2="16"/><line x1="20" y1="12" x2="20" y2="4"/><circle cx="4" cy="12" r="2.2" fill="currentColor"/><circle cx="12" cy="10" r="2.2" fill="currentColor"/><circle cx="20" cy="14" r="2.2" fill="currentColor"/></svg>"#,
            ),
            "icons/scope.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="4" width="19" height="13" rx="2.2"/><path d="M8 21h8M12 17v4"/><path d="M5.5 11.5h3l1.8-3.5 2.6 6 1.8-2.5h3.8"/></svg>"#,
            ),
            "icons/preview.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="4.5" width="18" height="15" rx="2.5"/><circle cx="8.5" cy="9.5" r="1.5" fill="currentColor"/><path d="m21 15.5-5-4.5-6.5 6.5"/></svg>"#,
            ),
            "icons/markup.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/></svg>"#,
            ),
            "icons/timer.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="10" y1="2" x2="14" y2="2"/><line x1="12" y1="14" x2="15.5" y2="10.5"/><circle cx="12" cy="14" r="8"/></svg>"#,
            ),
            "icons/gear.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M19.4 13a7.9 7.9 0 0 0 .1-1 7.9 7.9 0 0 0-.1-1l2.1-1.6a.5.5 0 0 0 .1-.6l-2-3.5a.5.5 0 0 0-.6-.2l-2.5 1a7.4 7.4 0 0 0-1.7-1l-.4-2.6a.5.5 0 0 0-.5-.5h-4a.5.5 0 0 0-.5.5l-.4 2.6a7.4 7.4 0 0 0-1.7 1l-2.5-1a.5.5 0 0 0-.6.2l-2 3.5a.5.5 0 0 0 .1.6L4.6 11a7.9 7.9 0 0 0-.1 1c0 .3 0 .7.1 1l-2.1 1.6a.5.5 0 0 0-.1.6l2 3.5a.5.5 0 0 0 .6.2l2.5-1a7.4 7.4 0 0 0 1.7 1l.4 2.6a.5.5 0 0 0 .5.5h4a.5.5 0 0 0 .5-.5l.4-2.6a7.4 7.4 0 0 0 1.7-1l2.5 1a.5.5 0 0 0 .6-.2l2-3.5a.5.5 0 0 0-.1-.6L19.4 13ZM12 15.5a3.5 3.5 0 1 1 0-7 3.5 3.5 0 0 1 0 7Z"/></svg>"#,
            ),
            "icons/pin.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 17v5"/><path d="M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.2V16h14v-.8a2 2 0 0 0-1.1-1.7l-1.8-.9a2 2 0 0 1-1.1-1.8V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1Z"/></svg>"#,
            ),
            "icons/close.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>"#,
            ),
            "icons/copy.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>"#,
            ),
            "icons/save.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 3v12m0 0-4-4m4 4 4-4M4 19h16"/></svg>"#,
            ),
            "icons/split.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="4" width="18" height="16" rx="2"/><line x1="12" y1="4" x2="12" y2="20"/></svg>"#,
            ),
            "icons/heat.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 3c2 4 6 5 6 10a6 6 0 1 1-12 0c0-2.5 1.5-4.5 3-6 0 2 1 3 2 3 0-3 1-5 1-7Z"/></svg>"#,
            ),
            "icons/reset.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/></svg>"#,
            ),
            "icons/grip.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M8 6l-4 6 4 6M16 6l4 6-4 6"/></svg>"#,
            ),
            "icons/box.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="4" y="5" width="16" height="14" rx="2"/></svg>"#,
            ),
            "icons/arrow.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 19 19 5M11 5h8v8"/></svg>"#,
            ),
            "icons/redact.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="6" width="18" height="12" rx="2"/><path d="m7 18 10-12"/></svg>"#,
            ),
            "icons/check.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M20 6 9 17l-5-5"/></svg>"#,
            ),
            "icons/keyboard.svg" => Some(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="2" y="5" width="20" height="14" rx="2"/><path d="M6 9h.01M10 9h.01M14 9h.01M18 9h.01M6 13h.01M10 13h.01M14 13h.01M18 13h.01M8 16h8"/></svg>"#,
            ),
            _ => None,
        };
        Ok(data.map(|s| Cow::Borrowed(s.as_bytes())))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![])
    }
}
