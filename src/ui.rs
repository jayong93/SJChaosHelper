use anyhow::{anyhow, Result};
use font_loader::system_fonts;
use helper::AccountData;
use iced::{self, widget, Color, Element};
use iced_native::Event;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    ptr::null_mut,
};
use winapi;

const SAVE_FILE_NAME: &'static str = "chaos_helper.info";
lazy_static! {
    static ref LEAGUE_DATA: Vec<String> = helper::get_league_list()
        .map_err(|e| error_message_box(e))
        .unwrap();
}

pub fn error_message_box(s: impl ToString) {
    use std::os::windows::ffi::*;
    let s: OsString = s.to_string().into();
    std::thread::spawn(move || {
        let mut s_it = s.encode_wide().collect::<Vec<_>>();
        s_it.push(0);
        let mut caption = OsStr::new("Error").encode_wide().collect::<Vec<_>>();
        caption.push(0);
        unsafe {
            winapi::um::winuser::MessageBoxW(
                null_mut(),
                s_it.as_ptr(),
                caption.as_ptr(),
                winapi::um::winuser::MB_OK | winapi::um::winuser::MB_SYSTEMMODAL,
            );
        }
    });
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct LeagueIdx(usize);

impl ToString for LeagueIdx {
    fn to_string(&self) -> String {
        LEAGUE_DATA[self.0].clone()
    }
}

#[derive(Clone, Debug)]
enum AppMessage {
    LabelUpdateStarted(usize),
    LabelUpdated { idx: usize, text: String },
    LabelUpdateCompleted(usize),
    LeagueUpdated(usize),
    StartHelper,
    SaveConfig,
    EventOccurred(Event),
}

#[derive(Debug)]
enum EditableLabel {
    Text(String, iced::button::State),
    Edit(String, iced::text_input::State),
}

struct Bordered;

impl widget::container::StyleSheet for Bordered {
    fn style(&self) -> widget::container::Style {
        widget::container::Style {
            border_width: 2.,
            border_color: Color::from_rgb(0.5, 0.5, 0.5),
            ..Default::default()
        }
    }
}

impl EditableLabel {
    fn view(&mut self, name: &str, idx: usize, font: iced::Font) -> Element<'_, AppMessage> {
        use iced::*;

        match self {
            Self::Text(text, state) => {
                let row = Row::new()
                    .spacing(20)
                    .align_items(Align::Center)
                    .width(Length::Fill);
                let row = row.push(Text::new(name).font(font));
                row.push(
                    Container::new(Text::new(&*text).font(font))
                        .padding(4)
                        .width(Length::Fill)
                        .style(Bordered),
                )
                .push(
                    Button::new(state, Text::new("Edit").font(font))
                        .on_press(AppMessage::LabelUpdateStarted(idx)),
                )
                .into()
            }
            Self::Edit(input, state) => {
                let row = Row::new()
                    .spacing(20)
                    .align_items(Align::Center)
                    .width(Length::Fill);
                let row = row.push(Text::new(name).font(font));
                row.push(
                    Container::new(
                        TextInput::new(state, "Input data", input, move |text| {
                            AppMessage::LabelUpdated { idx, text }
                        })
                        .font(font)
                        .width(Length::Fill)
                        .on_submit(AppMessage::LabelUpdateCompleted(idx)),
                    )
                    .padding(4)
                    .width(Length::Fill)
                    .style(Bordered),
                )
                .into()
            }
        }
    }
}

impl Default for EditableLabel {
    fn default() -> Self {
        Self::Edit(Default::default(), Default::default())
    }
}

#[derive(Debug, Eq, PartialEq)]
enum AdjustingWindowStatus {
    None,
    LeftTop,
    RightBottom,
}

#[derive(Debug)]
struct App {
    loop_proxy: crate::EventLoopProxy<crate::UIMessage>,
    account_data: AccountData,
    league: Option<usize>,
    league_picklist_state: widget::pick_list::State<LeagueIdx>,
    labels: [EditableLabel; 3],
    start_button_state: widget::button::State,
    save_button_state: widget::button::State,
    font: iced::Font,
    win_status: AdjustingWindowStatus,
    win_rect: Option<WindowRect>,
}

#[derive(Debug, Serialize, Deserialize, Copy, Clone)]
pub struct WindowRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Default for WindowRect {
    fn default() -> Self {
        Self {
            left: crate::STASH_POS.0 as _,
            top: crate::STASH_POS.1 as _,
            right: (crate::STASH_POS.0 + crate::STASH_SIZE.0) as _,
            bottom: (crate::STASH_POS.1 + crate::STASH_SIZE.1) as _,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SaveData {
    #[serde(flatten)]
    account_data: AccountData,
    window_size: Option<WindowRect>,
}

pub fn save_account_data(path: &std::path::Path, account: &SaveData) -> Result<()> {
    use serde_json::to_writer;
    use std::fs::OpenOptions;

    let out_file = OpenOptions::new()
        .truncate(true)
        .create(true)
        .write(true)
        .open(path)?;
    to_writer(out_file, account)?;

    Ok(())
}

pub fn load_account_data(path: &std::path::Path) -> Result<SaveData> {
    use serde_json::from_reader;
    use std::fs::OpenOptions;
    let out_file = OpenOptions::new().read(true).open(path)?;
    from_reader(out_file).map_err(|e| anyhow!(e))
}

impl App {
    const LABEL_NAMES: [&'static str; 3] = ["Account", "Cookie", "Tab Index"];
}

use iced::Command;
impl iced::Application for App {
    type Message = AppMessage;
    type Executor = iced::executor::Default;
    type Flags = (
        SaveData,
        crate::EventLoopProxy<crate::UIMessage>,
        iced::Font,
    );

    fn new(flag: Self::Flags) -> (Self, Command<Self::Message>) {
        let league_data = &LEAGUE_DATA;
        let league = league_data
            .iter()
            .enumerate()
            .find(|(_, league)| flag.0.account_data.league == **league)
            .map(|(idx, _)| idx);
        let labels = [
            EditableLabel::Text(flag.0.account_data.account.clone(), Default::default()),
            EditableLabel::Text(flag.0.account_data.cookie.clone(), Default::default()),
            EditableLabel::Text(flag.0.account_data.tab_idx.to_string(), Default::default()),
        ];
        (
            Self {
                loop_proxy: flag.1,
                account_data: flag.0.account_data,
                league,
                league_picklist_state: Default::default(),
                labels,
                start_button_state: Default::default(),
                save_button_state: Default::default(),
                font: flag.2,
                win_status: AdjustingWindowStatus::None,
                win_rect: flag.0.window_size,
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        "Chaos Helper".to_owned()
    }

    fn subscription(&self) -> iced::Subscription<Self::Message> {
        iced_native::subscription::events().map(AppMessage::EventOccurred)
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        match message {
            AppMessage::LabelUpdateStarted(idx) => {
                if let EditableLabel::Text(text, _) = &self.labels[idx] {
                    self.labels[idx] =
                        EditableLabel::Edit(text.clone(), widget::text_input::State::focused());
                }
            }
            AppMessage::LabelUpdated { idx, text } => {
                if let EditableLabel::Edit(t, _) = &mut self.labels[idx] {
                    *t = text;
                }
            }
            AppMessage::LabelUpdateCompleted(idx) => {
                if let EditableLabel::Edit(text, _) = &self.labels[idx] {
                    match idx {
                        0 => {
                            self.account_data.account = text.clone();
                            self.labels[idx] =
                                EditableLabel::Text(text.clone(), Default::default());
                        }
                        1 => {
                            self.account_data.cookie = text.clone();
                            self.labels[idx] =
                                EditableLabel::Text(text.clone(), Default::default());
                        }
                        2 => {
                            if let Ok(tab_idx) = text.parse::<usize>() {
                                self.account_data.tab_idx = tab_idx;
                            }
                            self.labels[idx] = EditableLabel::Text(
                                self.account_data.tab_idx.to_string(),
                                Default::default(),
                            );
                        }
                        _ => unreachable!(),
                    }
                }
            }
            AppMessage::LeagueUpdated(idx) => {
                self.league = Some(idx);
                self.account_data.league = LEAGUE_DATA[idx].clone();
            }
            AppMessage::StartHelper => {
                helper::set_account(self.account_data.clone());
                crate::IS_INITIALIZED.store(true, std::sync::atomic::Ordering::Relaxed);
                if let Err(e) = self.loop_proxy.send_event(crate::UIMessage::ShowStatus) {
                    error_message_box(e);
                }
            }
            AppMessage::SaveConfig => {
                let save_name = dirs::home_dir()
                    .unwrap_or_else(|| {
                        error_message_box("사용자 폴더의 위치를 불러올 수 없습니다.");
                        panic!("사용자 폴더의 위치를 불러올 수 없습니다.")
                    })
                    .join(SAVE_FILE_NAME);
                let save_data = SaveData {
                    account_data: self.account_data.clone(),
                    window_size: self.win_rect,
                };
                if let Err(e) = save_account_data(&save_name, &save_data) {
                    error_message_box(e);
                }
            }
            AppMessage::EventOccurred(event) => {
                use iced_native::{device, keyboard};
                use keyboard::KeyCode;
                match event {
                    Event::Raw(device::Event::KeyInput(keyboard::Event::KeyPressed {
                        key_code,
                        modifiers,
                    })) => match key_code {
                        _ if !crate::IS_INITIALIZED.load(std::sync::atomic::Ordering::Acquire)
                            || !modifiers.control
                            || !modifiers.shift => {}
                        KeyCode::F8 if self.win_status == AdjustingWindowStatus::LeftTop => {
                            if let Ok((cx, cy)) = crate::get_cursor_pos() {
                                let mut rect = self.win_rect.unwrap_or_default();
                                rect.left = cx;
                                rect.top = cy;
                                self.win_rect = Some(rect);
                            }
                            self.win_status = AdjustingWindowStatus::RightBottom;
                        }
                        KeyCode::F8 if self.win_status == AdjustingWindowStatus::RightBottom => {
                            if let Ok((cx, cy)) = crate::get_cursor_pos() {
                                let mut rect = self.win_rect.unwrap_or_default();
                                rect.right = cx;
                                rect.bottom = cy;
                                self.win_rect = Some(rect);
                            }
                            self.win_status = AdjustingWindowStatus::None;
                        }
                        KeyCode::F8 => {
                            self.win_status = AdjustingWindowStatus::LeftTop;
                        }
                        KeyCode::F9 => {
                            if let Err(e) =
                                self.loop_proxy.send_event(crate::UIMessage::ShowStashMask)
                            {
                                error_message_box(e);
                            }
                        }
                        KeyCode::F10 => {
                            if let Err(e) = self.loop_proxy.send_event(crate::UIMessage::ShowStatus)
                            {
                                error_message_box(e);
                            }
                        }
                        KeyCode::F11 => {
                            if let Err(e) =
                                self.loop_proxy.send_event(crate::UIMessage::CloseWindow)
                            {
                                error_message_box(e);
                            }
                        }
                        _ => {}
                    },
                    Event::Raw(iced_native::device::Event::MouseMotion(_x, _y)) => {
                        let result = match self.win_status {
                            AdjustingWindowStatus::LeftTop => {
                                self.loop_proxy.send_event(crate::UIMessage::ChangeLeftTop)
                            }
                            AdjustingWindowStatus::RightBottom => self
                                .loop_proxy
                                .send_event(crate::UIMessage::ChangeRightBottom),
                            _ => Ok(()),
                        };

                        if let Err(e) = result {
                            error_message_box(e);
                        }
                    }
                    _ => {}
                }
            }
        }
        Command::none()
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        use iced::*;

        let font = self.font;

        let radio_row = Row::new()
            .padding(20)
            .spacing(20)
            .align_items(Align::Center)
            .width(Length::Fill)
            .push(Text::new("League").font(font));

        let league_data = &LEAGUE_DATA;
        let selected_league = self.league.map(|selected_idx| LeagueIdx(selected_idx));
        let league_indices: Vec<_> = (0..league_data.len()).map(|idx| LeagueIdx(idx)).collect();
        let radio_row = radio_row.push(
            PickList::new(
                &mut self.league_picklist_state,
                league_indices,
                selected_league,
                |idx| AppMessage::LeagueUpdated(idx.0),
            )
            .width(Length::Fill),
        );

        let column = Column::new().spacing(20).align_items(Align::Center);
        let column = column.push(radio_row);
        let column = self
            .labels
            .iter_mut()
            .enumerate()
            .fold(column, |col, (idx, label)| {
                let row = Row::new().padding(20).align_items(Align::Center);
                let row = row.push(label.view(Self::LABEL_NAMES[idx], idx, font));

                col.push(row)
            });

        column
            .push(
                Container::new(
                    Row::new()
                        .spacing(20)
                        .align_items(Align::Center)
                        .push(
                            Button::new(&mut self.start_button_state, Text::new("실행").font(font))
                                .width(Length::Shrink)
                                .on_press(AppMessage::StartHelper),
                        )
                        .push(
                            Button::new(
                                &mut self.save_button_state,
                                Text::new("설정 저장").font(font),
                            )
                            .width(Length::Shrink)
                            .on_press(AppMessage::SaveConfig),
                        ),
                )
                .width(Length::Fill)
                .align_x(Align::Center),
            )
            .into()
    }
}

pub fn run_ui(loop_proxy: crate::EventLoopProxy<crate::UIMessage>) -> Result<()> {
    use iced::Application;

    let save_name = dirs::home_dir()
        .unwrap_or_else(|| {
            error_message_box("사용자 폴더의 위치를 불러올 수 없습니다.");
            panic!("사용자 폴더의 위치를 불러올 수 없습니다.")
        })
        .join(SAVE_FILE_NAME);
    let save_data = load_account_data(&save_name)
        .map_err(|e| error_message_box(e))
        .unwrap_or_default();

    loop_proxy
        .send_event(crate::UIMessage::InitWindow(
            save_data.window_size.unwrap_or_default(),
        ))
        .unwrap();

    let mut font_property = system_fonts::FontPropertyBuilder::new()
        .family("맑은 고딕")
        .build();
    let font = if let Some(font) =
        system_fonts::get(&mut font_property).map(|(data, _idx)| Box::leak(data.into_boxed_slice()))
    {
        iced::Font::External {
            name: "맑은 고딕",
            bytes: font,
        }
    } else {
        iced::Font::Default
    };

    App::run(iced::Settings::with_flags((save_data, loop_proxy, font))).unwrap();
    Ok(())
}
