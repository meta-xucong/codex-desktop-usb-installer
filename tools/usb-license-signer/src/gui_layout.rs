//! Layout only: logical Win32 coordinates (Windows retains DPI virtualization).

pub const ID_TARGETS: i32 = 1001;
pub const ID_REFRESH: i32 = 1002;
pub const ID_SUMMARY: i32 = 1007;
pub const ID_STATUS: i32 = 1008;
pub const ID_PROVISION_SIGN: i32 = 1009;
pub const ID_BASE_URL: i32 = 1010;
pub const ID_ADMIN_KEY: i32 = 1011;
pub const ID_EMAIL_DOMAIN: i32 = 1013;
pub const ID_PRELOAD: i32 = 1014;
pub const ID_RETRY_PRELOAD: i32 = 1015;
pub const ID_ACCOUNT_PASSWORD: i32 = 1016;
pub const ID_REGISTER: i32 = 1017;
pub const ID_TITLE: i32 = 2001;
pub const ID_INTRO: i32 = 2002;
pub const ID_AUTHENTICODE: i32 = 2003;
pub const ID_TARGET_LABEL: i32 = 2004;
pub const ID_ACCOUNT_LABEL: i32 = 2005;
pub const ID_BASE_LABEL: i32 = 2006;
pub const ID_ADMIN_LABEL: i32 = 2007;
pub const ID_GROUP_LABEL: i32 = 2008;
pub const ID_GROUP_VALUE: i32 = 2009;
pub const ID_EMAIL_LABEL: i32 = 2010;
pub const ID_PASSWORD_LABEL: i32 = 2011;
pub const ID_PRELOAD_LABEL: i32 = 2012;
pub const ID_SUMMARY_LABEL: i32 = 2013;

#[derive(Debug)]
pub struct Placement {
    pub id: i32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub controls: Vec<Placement>,
}

pub fn calculate(client_width: i32, client_height: i32) -> Layout {
    let width = client_width.max(360);
    let inner = width - 32;
    let wide = width >= 720;
    let mut controls = Vec::new();
    let mut put = |id, x, y, width, height| {
        controls.push(Placement {
            id,
            x,
            y,
            width,
            height,
        });
    };
    put(ID_TITLE, 16, 12, inner, 28);
    let mut y = 44;
    let intro_height = if wide { 24 } else { 44 };
    put(ID_INTRO, 16, y, inner, intro_height);
    y += intro_height;
    let auth_height = if width >= 540 { 22 } else { 38 };
    put(ID_AUTHENTICODE, 16, y, inner, auth_height);
    y += auth_height + 8;
    put(ID_TARGET_LABEL, 16, y, inner, 22);
    y += 24;
    put(ID_TARGETS, 16, y, inner - 108, 28);
    put(ID_REFRESH, width - 116, y, 100, 28);
    y += 40;
    let account_height = if wide { 24 } else { 44 };
    put(ID_ACCOUNT_LABEL, 16, y, inner, account_height);
    y += account_height + 4;

    let fields = [
        (ID_BASE_LABEL, ID_BASE_URL),
        (ID_ADMIN_LABEL, ID_ADMIN_KEY),
        (ID_GROUP_LABEL, ID_GROUP_VALUE),
        (ID_EMAIL_LABEL, ID_EMAIL_DOMAIN),
        (ID_PASSWORD_LABEL, ID_ACCOUNT_PASSWORD),
        (ID_PRELOAD_LABEL, ID_PRELOAD),
    ];
    let column_width = if wide { (inner - 16) / 2 } else { inner };
    for (index, (label, value)) in fields.into_iter().enumerate() {
        let column = if wide { index % 2 } else { 0 } as i32;
        let row = if wide { index / 2 } else { index } as i32;
        let x = 16 + column * (column_width + 16);
        let field_y = y + row * 56;
        put(label, x, field_y, column_width, 22);
        put(value, x, field_y + 22, column_width, 26);
    }
    y += if wide { 3 * 56 } else { 6 * 56 };
    put(ID_SUMMARY_LABEL, 16, y, inner, 22);
    y += 24;
    let button_rows = if width >= 620 { 1 } else { 3 };
    let footer = 8 + 48 + 8 + button_rows * 46 + 8;
    let summary_height = (client_height - y - footer).max(104);
    put(ID_SUMMARY, 16, y, inner, summary_height);
    y += summary_height + 8;
    put(ID_STATUS, 16, y, inner, 48);
    y += 56;
    let buttons = [ID_PROVISION_SIGN, ID_REGISTER, ID_RETRY_PRELOAD];
    let button_width = if button_rows == 1 {
        (inner - 16) / 3
    } else {
        inner
    };
    for (index, id) in buttons.into_iter().enumerate() {
        let x = if button_rows == 1 {
            16 + index as i32 * (button_width + 8)
        } else {
            16
        };
        let button_y = if button_rows == 1 {
            y
        } else {
            y + index as i32 * 46
        };
        put(id, x, button_y, button_width, 38);
    }
    Layout {
        width,
        height: y + button_rows * 46 + 8,
        controls,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_reachable_without_overlap_at_resolution_and_scale_matrix() {
        for (screen_width, screen_height) in [
            (800, 600),
            (1024, 768),
            (1280, 720),
            (1366, 768),
            (1920, 1080),
        ] {
            for scale in [1.0_f64, 1.25, 1.5, 2.0] {
                // Allow for taskbar, window frame and native scrollbars.
                let width = ((screen_width as f64 / scale) as i32 - 56).min(780);
                let height = ((screen_height as f64 / scale) as i32 - 112).min(700);
                let layout = calculate(width, height);
                assert_eq!(layout.controls.len(), 25);
                for (index, a) in layout.controls.iter().enumerate() {
                    assert!(a.width > 0 && a.height > 0);
                    assert!(a.x >= 0 && a.y >= 0);
                    assert!(a.x + a.width <= layout.width && a.y + a.height <= layout.height);
                    // Native scroll ranges must let every control be brought fully into view.
                    assert!(a.width <= width && a.height <= height);
                    for b in &layout.controls[index + 1..] {
                        assert_ne!(a.id, b.id);
                        assert!(
                            a.x + a.width <= b.x
                                || b.x + b.width <= a.x
                                || a.y + a.height <= b.y
                                || b.y + b.height <= a.y,
                            "overlap: {a:?} {b:?} at {width}x{height}"
                        );
                    }
                }
            }
        }
    }
}
