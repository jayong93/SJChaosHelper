// #![windows_subsystem = "windows"]
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use winapi::shared::minwindef::FALSE;
use winapi::shared::ntdef::NULL;
use winapi::shared::windef::{HWND__, RECT};
use winapi::um::wingdi::{self, RGB};
use winapi::um::winuser;
use winit::{
    dpi::LogicalPosition,
    event::{DeviceEvent, Event, VirtualKeyCode, WindowEvent},
    event_loop::EventLoopProxy,
    platform::windows::{EventLoopExtWindows, WindowExtWindows},
    *,
};

mod ui;

const STASH_SIZE: (u32, u32) = (632, 632);
const STASH_POS: (u32, u32) = (17, 162);
static IS_INITIALIZED: AtomicBool = AtomicBool::new(false);

fn get_item_rect(mut x: u32, mut y: u32, mut w: u32, mut h: u32, is_quad_stash: bool) -> RECT {
    if !is_quad_stash {
        x *= 2;
        y *= 2;
        w *= 2;
        h *= 2;
    }
    let left = ((x * 26) + (x / 3)) as _;
    let top = ((y * 26) + (y / 3)) as _;
    let right = ((x + w) * 26 + ((x + w) / 3)) as _;
    let bottom = ((y + h) * 26 + ((y + h) / 3)) as _;
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

#[derive(Debug)]
pub enum UIMessage {
    CloseWindow,
    ShowStatus,
    ShowStashMask,
    ShowResult(helper::ResponseFromNetwork),
}

fn main() -> Result<()> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStrExt;
    helper::init_module();

    let (tx, rx) = std::sync::mpsc::channel::<EventLoopProxy<UIMessage>>();
    let handle = std::thread::spawn(move || -> Result<()> {
        let event_loop = event_loop::EventLoop::new_any_thread();
        let loop_proxy = event_loop.create_proxy();
        tx.send(loop_proxy.clone()).unwrap();
        let main_window = window::WindowBuilder::new()
            .with_always_on_top(true)
            .with_resizable(false)
            .with_visible(false)
            .build(&event_loop)?;

        main_window.set_outer_position(LogicalPosition::new(STASH_POS.0, STASH_POS.1));
        let main_hwnd = main_window.hwnd() as *mut HWND__;
        let mut main_rect = RECT {
            top: 0,
            left: 0,
            bottom: STASH_SIZE.1 as _,
            right: STASH_SIZE.0 as _,
        };
        unsafe {
            let style = winuser::GetWindowLongA(main_hwnd, winuser::GWL_STYLE);
            let main_style = style
                & !(winuser::WS_OVERLAPPED as i32
                    | winuser::WS_SYSMENU as i32
                    | winuser::WS_CAPTION as i32);
            winuser::SetWindowLongA(main_hwnd, winuser::GWL_STYLE, main_style);
            winuser::AdjustWindowRect(&mut main_rect, main_style as _, FALSE);
            winuser::SetWindowPos(
                main_hwnd,
                NULL as _,
                0,
                0,
                main_rect.right - main_rect.left,
                main_rect.bottom - main_rect.top,
                winuser::SWP_NOMOVE
                    | winuser::SWP_NOACTIVATE
                    | winuser::SWP_NOZORDER
                    | winuser::SWP_NOOWNERZORDER,
            );
        }
        set_main_window_style(main_hwnd);

        event_loop.run(move |event, _, control_flow| {
            *control_flow = event_loop::ControlFlow::Wait;

            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    window_id,
                } if window_id == main_window.id() => {
                    *control_flow = event_loop::ControlFlow::Exit;
                }
                Event::UserEvent(e) => {
                    show_window(main_hwnd);
                    match e {
                        UIMessage::CloseWindow => unsafe {
                            winuser::ShowWindow(main_hwnd, winuser::SW_HIDE);
                        },
                        UIMessage::ShowStashMask => {
                            if let Ok(result) = helper::acquire_chaos_list(false) {
                                loop_proxy.send_event(UIMessage::ShowResult(result)).ok();
                            }
                        }
                        UIMessage::ShowStatus => {
                            if let Ok(result) = helper::acquire_chaos_list(true) {
                                loop_proxy.send_event(UIMessage::ShowResult(result)).ok();
                            }
                        }
                        UIMessage::ShowResult(result) => match result {
                            helper::ResponseFromNetwork::StashStatus((recipe_map, chaos_num)) => {
                                toggle_window_transparent(main_hwnd, true);
                                let types = [
                                    helper::ItemType::Weapon1HOrShield,
                                    helper::ItemType::Weapon2H,
                                    helper::ItemType::Body,
                                    helper::ItemType::Helmet,
                                    helper::ItemType::Gloves,
                                    helper::ItemType::Belt,
                                    helper::ItemType::Boots,
                                    helper::ItemType::Ring,
                                    helper::ItemType::Amulet,
                                ];

                                let mut info =
                                    OsString::from("--- Type: (ilvl<75, ilvl>=75) ---\n");
                                for item_type in types.iter() {
                                    let (chaos, regal) = recipe_map
                                        .get(item_type)
                                        .map(|(c, r)| (c.len(), r.len()))
                                        .unwrap_or((0, 0));
                                    info.push(format!(
                                        "{}: ({}, {})\n",
                                        item_type.as_ref(),
                                        chaos,
                                        regal
                                    ));
                                }
                                info.push(format!("Total Chaos: {}", chaos_num));

                                let text: Vec<_> = info.encode_wide().collect();
                                let mut text_rect = main_rect.clone();
                                unsafe {
                                    let main_dc = winuser::GetDC(main_hwnd);
                                    winuser::DrawTextW(
                                        main_dc,
                                        text.as_ptr(),
                                        text.len() as i32,
                                        &mut text_rect,
                                        winuser::DT_CALCRECT
                                            | winuser::DT_WORDBREAK
                                            | winuser::DT_CENTER
                                            | winuser::DT_VCENTER,
                                    );

                                    let green_brush = wingdi::CreateSolidBrush(RGB(0, 255, 0));
                                    let white_brush =
                                        wingdi::GetStockObject(wingdi::WHITE_BRUSH as i32);
                                    winuser::FillRect(main_dc, &main_rect, green_brush as _);
                                    winuser::FillRect(main_dc, &text_rect, white_brush as _);
                                    winuser::DrawTextW(
                                        main_dc,
                                        text.as_ptr(),
                                        text.len() as i32,
                                        &mut text_rect,
                                        winuser::DT_CENTER
                                            | winuser::DT_VCENTER
                                            | winuser::DT_WORDBREAK,
                                    );

                                    wingdi::DeleteObject(green_brush as _);
                                    winuser::ReleaseDC(main_hwnd, main_dc);
                                }
                            }
                            helper::ResponseFromNetwork::ChaosRecipe((
                                chaos_recipe,
                                is_quad_stash,
                            )) => {
                                let main_dc;
                                unsafe {
                                    main_dc = winuser::GetDC(main_hwnd);
                                    let white_brush =
                                        wingdi::GetStockObject(wingdi::WHITE_BRUSH as _);
                                    winuser::FillRect(main_dc, &main_rect, white_brush as _);
                                }

                                if chaos_recipe.is_empty() {
                                    toggle_window_transparent(main_hwnd, true);
                                    let text = OsString::from("카오스 레시피가 없습니다")
                                        .encode_wide()
                                        .collect::<Vec<_>>();
                                    unsafe {
                                        winuser::DrawTextW(
                                            main_dc,
                                            text.as_ptr(),
                                            text.len() as _,
                                            &mut main_rect,
                                            winuser::DT_CENTER
                                                | winuser::DT_VCENTER
                                                | winuser::DT_SINGLELINE,
                                        );
                                    }
                                } else {
                                    toggle_window_transparent(main_hwnd, false);
                                    unsafe {
                                        let brush = wingdi::CreateSolidBrush(RGB(0, 255, 0));

                                        for recipe in chaos_recipe.iter() {
                                            let (x, y) = (recipe.x as u32, recipe.y as u32);
                                            let (w, h) = (recipe.w as u32, recipe.h as u32);

                                            let rect = get_item_rect(x, y, w, h, is_quad_stash);

                                            winuser::FillRect(main_dc, &rect, brush);
                                        }
                                        wingdi::DeleteObject(brush as _);
                                    }
                                }
                                unsafe {
                                    winuser::ReleaseDC(main_hwnd, main_dc);
                                }
                            }
                        },
                    }
                }
                _ => {}
            }
        })
    });

    let loop_proxy = rx.recv()?;

    ui::run_ui(loop_proxy)
}

fn toggle_window_transparent(hwnd: *mut HWND__, apply: bool) {
    unsafe {
        let style = winuser::GetWindowLongA(hwnd, winuser::GWL_EXSTYLE);
        let style = if apply {
            style | winuser::WS_EX_TRANSPARENT as i32 | winuser::WS_EX_TOOLWINDOW as i32
        } else {
            style & !winuser::WS_EX_TRANSPARENT as i32 | winuser::WS_EX_TOOLWINDOW as i32
        };
        winuser::SetWindowLongA(hwnd, winuser::GWL_EXSTYLE, style);
    }
}

fn set_main_window_style(hwnd: *mut HWND__) {
    unsafe {
        winuser::SetWindowLongA(
            hwnd,
            winuser::GWL_EXSTYLE,
            winuser::WS_EX_LAYERED as i32
                | winuser::WS_EX_TRANSPARENT as i32
                | winuser::WS_EX_TOOLWINDOW as i32,
        );
        winuser::SetLayeredWindowAttributes(
            hwnd,
            RGB(0, 255, 0),
            175,
            winuser::LWA_ALPHA | winuser::LWA_COLORKEY,
        );
        let style = winuser::GetWindowLongA(hwnd, winuser::GWL_STYLE);
        let main_style = style
            & !(winuser::WS_OVERLAPPED as i32
                | winuser::WS_SYSMENU as i32
                | winuser::WS_CAPTION as i32);
        winuser::SetWindowLongA(hwnd, winuser::GWL_STYLE, main_style);
    }
}

fn show_window(hwnd: *mut HWND__) {
    unsafe {
        winuser::ShowWindow(hwnd, winuser::SW_SHOWNA);
    }
}
