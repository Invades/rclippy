use std::sync::Once;

use windows_sys::{
    Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA},
    s,
};

static INIT_DARK_MENUS: Once = Once::new();

pub fn enable_dark_menus_if_supported() {
    INIT_DARK_MENUS.call_once(|| unsafe {
        let uxtheme = LoadLibraryA(s!("uxtheme.dll"));
        if uxtheme.is_null() {
            return;
        }

        const UXTHEME_SET_PREFERRED_APP_MODE: usize = 135;
        const UXTHEME_FLUSH_MENU_THEMES: usize = 136;
        const PREFERRED_APP_MODE_ALLOW_DARK: i32 = 1;

        type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
        type FlushMenuThemes = unsafe extern "system" fn();

        if let Some(set_preferred_app_mode) =
            GetProcAddress(uxtheme, UXTHEME_SET_PREFERRED_APP_MODE as *const u8)
                .map(|proc| std::mem::transmute::<_, SetPreferredAppMode>(proc))
        {
            let _ = set_preferred_app_mode(PREFERRED_APP_MODE_ALLOW_DARK);
        }

        if let Some(flush_menu_themes) =
            GetProcAddress(uxtheme, UXTHEME_FLUSH_MENU_THEMES as *const u8)
                .map(|proc| std::mem::transmute::<_, FlushMenuThemes>(proc))
        {
            flush_menu_themes();
        }
    });
}
