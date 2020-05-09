use anyhow::Result;
use winapi::shared::windef::{HWND__, RECT};
use winapi::um::wingdi::{self, RGB};
use winapi::um::winuser;
use winit::{
    dpi::{LogicalPosition, PhysicalSize},
    event::{DeviceEvent, Event, VirtualKeyCode, WindowEvent},
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    platform::windows::{EventLoopExtWindows, WindowExtWindows},
    *,
};

mod ui;

const QUAD_SIZE: u32 = 24;
const NORMAL_SIZE: u32 = 12;

fn make_child_window(
    main_window: &window::Window,
    width: u32,
    height: u32,
    event_loop: &EventLoopWindowTarget<helper::ResponseFromNetwork>,
) -> Result<window::Window> {
    let child = window::WindowBuilder::new()
        .with_decorations(false)
        .with_resizable(false)
        .with_visible(true)
        .with_inner_size(PhysicalSize::new(width, height))
        .build(&event_loop)?;

    let main_hwnd = main_window.hwnd() as *mut HWND__;
    unsafe {
        winuser::SetParent(child.hwnd() as _, main_hwnd);
    }
    child.set_outer_position(LogicalPosition::new(0, 0));
    Ok(child)
}

fn main() -> Result<()> {
    helper::init_module();

    let (tx, rx) = std::sync::mpsc::channel::<EventLoopProxy<helper::ResponseFromNetwork>>();
    std::thread::spawn(move || -> Result<()> {
        let event_loop = event_loop::EventLoop::new_any_thread();
        let loop_proxy = event_loop.create_proxy();
        tx.send(loop_proxy.clone()).unwrap();
        let main_window = window::WindowBuilder::new()
            .with_always_on_top(true)
            .with_resizable(false)
            .with_visible(false)
            .with_inner_size(PhysicalSize::new(650 - 32, 795 - 200))
            .build(&event_loop)?;
        main_window.set_outer_position(LogicalPosition::new(16, 160));
        let main_hwnd = main_window.hwnd() as *mut HWND__;
        let mut main_rect = RECT::default();
        unsafe {
            winuser::GetClientRect(main_hwnd, &mut main_rect);
        }

        let mut child_map = std::collections::HashMap::new();

        event_loop.run(move |event, loop_ref, control_flow| {
            *control_flow = event_loop::ControlFlow::Wait;

            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    window_id,
                } if window_id == main_window.id() => {
                    *control_flow = event_loop::ControlFlow::Exit;
                }
                Event::RedrawRequested(id) if id != main_window.id() => unsafe {
                    let mut rect = RECT::default();
                    let mut ps = winuser::PAINTSTRUCT::default();
                    let child: &window::Window = child_map.get(&id).unwrap();
                    let child_hwnd = child.hwnd() as _;
                    let child_dc = winuser::BeginPaint(child_hwnd, &mut ps as _);
                    winuser::GetClientRect(child_hwnd, &mut rect);
                    let brush = wingdi::CreateSolidBrush(RGB(0, 255, 0));
                    winuser::FillRect(child_dc, &rect, brush);
                    wingdi::DeleteObject(brush as _);
                    winuser::EndPaint(child_hwnd, &ps);
                },
                Event::DeviceEvent {
                    event: DeviceEvent::Key(key_event),
                    ..
                } => match key_event.virtual_keycode {
                    _ if !key_event.modifiers.ctrl() || !key_event.modifiers.shift() => {}
                    Some(VirtualKeyCode::F9) => {
                        if let Ok(result) = helper::acquire_chaos_list(false) {
                            loop_proxy.send_event(result).ok();
                        }
                    }
                    Some(VirtualKeyCode::F10) => unsafe {
                        winuser::ShowWindow(main_hwnd, winuser::SW_HIDE);
                    },
                    Some(VirtualKeyCode::F11) => {
                        if let Ok(result) = helper::acquire_chaos_list(true) {
                            loop_proxy.send_event(result).ok();
                        }
                    }
                    Some(VirtualKeyCode::F12) => unsafe {
                        winuser::ShowWindow(main_hwnd, winuser::SW_HIDE);
                    },
                    _ => {}
                },
                Event::UserEvent(e) => {
                    match e {
                        helper::ResponseFromNetwork::StashStatus((recipe_map, chaos_num)) => {
                            use std::ffi::OsString;
                            use std::os::windows::ffi::OsStrExt;

                            for child in child_map.values() {
                                let child_hwnd = child.hwnd() as _;
                                unsafe {
                                    winuser::ShowWindow(child_hwnd, winuser::SW_HIDE);
                                }
                            }

                            let types = [
                                helper::ItemType::Weapon1HOrShield,
                                helper::ItemType::Weapon2H,
                                helper::ItemType::Body,
                                helper::ItemType::Helmet,
                                helper::ItemType::Gloves,
                                helper::ItemType::Body,
                                helper::ItemType::Boots,
                                helper::ItemType::Ring,
                                helper::ItemType::Amulet,
                            ];

                            let mut info = OsString::from("Type: <75, >=75\n");
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
                            unsafe {
                                let main_dc = winuser::GetDC(main_hwnd);
                                let white_brush =
                                    wingdi::GetStockObject(wingdi::WHITE_BRUSH as i32);
                                winuser::FillRect(main_dc, &main_rect, white_brush as _);
                                winuser::DrawTextW(
                                    main_dc,
                                    text.as_ptr(),
                                    text.len() as i32,
                                    &mut main_rect,
                                    winuser::DT_CENTER
                                        | winuser::DT_VCENTER
                                        | winuser::DT_WORDBREAK,
                                );
                                winuser::ReleaseDC(main_hwnd, main_dc);
                            }
                        }
                        helper::ResponseFromNetwork::ChaosRecipe((chaos_recipe, is_quad_stash)) => {
                            unsafe {
                                let main_dc = winuser::GetDC(main_hwnd);
                                let white_brush =
                                    wingdi::GetStockObject(wingdi::WHITE_BRUSH as i32);
                                winuser::FillRect(main_dc, &main_rect, white_brush as _);
                                winuser::ReleaseDC(main_hwnd, main_dc);
                            }

                            if chaos_recipe.is_empty() {
                                for child in child_map.values() {
                                    let child_hwnd = child.hwnd() as _;
                                    unsafe {
                                        winuser::ShowWindow(child_hwnd, winuser::SW_HIDE);
                                    }
                                }
                            }

                            let main_size = main_window.inner_size();
                            let child_size = if is_quad_stash {
                                (main_size.width / QUAD_SIZE, main_size.height / QUAD_SIZE)
                            } else {
                                (
                                    main_size.width / NORMAL_SIZE,
                                    main_size.height / NORMAL_SIZE,
                                )
                            };

                            if chaos_recipe.len() > child_map.len() {
                                if let Ok(child) = make_child_window(
                                    &main_window,
                                    child_size.0,
                                    child_size.1,
                                    loop_ref,
                                ) {
                                    child_map.insert(child.id(), child);
                                }
                            }

                            for (recipe, child) in chaos_recipe.iter().zip(child_map.values()) {
                                let (x, y) = (recipe.x as u32, recipe.y as u32);
                                let (w, h) = (recipe.w as u32, recipe.h as u32);
                                child.set_inner_size(PhysicalSize::new(
                                    w * child_size.0,
                                    h * child_size.1,
                                ));
                                child.set_outer_position(LogicalPosition::new(
                                    x * child_size.0,
                                    y * child_size.1,
                                ));
                                child.set_visible(true);
                            }
                        }
                    }
                    set_window_transparent(main_hwnd);
                }
                _ => {}
            }
        })
    });

    let loop_proxy = rx.recv()?;

    let result = ui::init_ui();
    match result {
        Ok((mut terminal, account_data)) => {
            let result = ui::ui_loop(&mut terminal, account_data, loop_proxy);
            ui::close_ui(&mut terminal);
            result
        }
        Err(e) => Err(e),
    }
}

fn set_window_transparent(hwnd: *mut HWND__) {
    unsafe {
        let style = winuser::GetWindowLongA(hwnd, winuser::GWL_EXSTYLE);
        winuser::SetWindowLongA(
            hwnd,
            winuser::GWL_EXSTYLE,
            style
                | winuser::WS_EX_LAYERED as i32
                | winuser::WS_EX_TRANSPARENT as i32
                | winuser::WS_EX_TOOLWINDOW as i32,
        );
        let style = winuser::GetWindowLongA(hwnd, winuser::GWL_STYLE);
        winuser::SetLayeredWindowAttributes(hwnd, 0, 175, winuser::LWA_ALPHA);
        winuser::SetWindowLongA(
            hwnd,
            winuser::GWL_STYLE,
            style
                & !(winuser::WS_OVERLAPPED as i32
                    | winuser::WS_SYSMENU as i32
                    | winuser::WS_CAPTION as i32),
        );
        winuser::ShowWindow(hwnd, winuser::SW_SHOWNA);
    }
}
