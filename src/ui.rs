use anyhow::{anyhow, Result};
use clipboard::{self, ClipboardProvider};
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use helper::AccountData;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::{stdout, Stdout, Write};
use std::sync::Mutex;
use tui::{
    backend::{self, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    terminal::Frame,
    widgets::{Block, Borders, Paragraph, Text},
    Terminal,
};

const SAVE_FILE_NAME: &'static str = "chaos_helper.info";
lazy_static! {
    static ref LEAGUE_DATA: Result<Vec<String>> = helper::get_league_list();
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

pub fn init_ui() -> Result<(Terminal<CrosstermBackend<Stdout>>, SaveData)> {
    enable_raw_mode().expect("Can't use raw mode");
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).expect("Can't enter to alternate screen");
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("Can't create a terminal");
    terminal.hide_cursor().expect("Can't hide a cursor");

    let save_name = dirs::home_dir()
        .ok_or(anyhow!("Can't get home directory path"))?
        .join(SAVE_FILE_NAME);
    let error;
    let data;
    match load_account_data(&save_name) {
        Ok(account) => {
            data = account;
            error = "An saved file has been loaded successfully".to_string();
        }
        Err(_) => {
            error = "Can't load from a save file, use a default setting".to_owned();
            data = Default::default();
        }
    }

    let league_data = LEAGUE_DATA.as_ref().map_err(|e| anyhow!(e))?;

    terminal
        .draw(|mut f| {
            draw_ui(&mut f, &State::Show, &data, league_data, &error);
        })
        .map_err(|e| {
            close_ui(&mut terminal);
            e
        })?;

    Ok((terminal, data))
}

pub fn close_ui(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    disable_raw_mode().expect("Can't disable raw mode");
    execute!(terminal.backend_mut(), LeaveAlternateScreen).expect("Can't leave alternate screen");
    terminal.show_cursor().expect("Can't show cursor");
}

#[derive(Debug)]
enum Field {
    Account(String),
    Cookie(String),
    League(String),
    TabIdx(Option<u32>),
}

impl Field {
    fn handle_input(&mut self, input: char) {
        match self {
            Self::Account(s) | Self::Cookie(s) | Self::League(s) => {
                s.push(input);
            }
            Self::TabIdx(i) => {
                if let Some(digit) = input.to_digit(10) {
                    if let Some(idx) = i {
                        *idx *= 10;
                        *idx += digit;
                    } else {
                        *i = Some(digit);
                    }
                }
            }
        }
    }
    fn erase(&mut self) {
        match self {
            Self::Account(s) | Self::Cookie(s) | Self::League(s) => {
                s.pop();
            }
            Self::TabIdx(i) => {
                if let Some(idx) = i {
                    *idx /= 10;
                }
            }
        }
    }
    fn erase_word(&mut self) {
        use regex::Regex;
        lazy_static! {
            static ref LAST_WORD_PTN: Regex = Regex::new(r"(^|\s)\S+\s*$").unwrap();
        }

        match self {
            Self::Account(s) | Self::Cookie(s) | Self::League(s) => {
                *s = LAST_WORD_PTN.replace(s, "$1").into_owned();
            }
            Self::TabIdx(i) => *i = None,
        }
    }
}
#[derive(Debug)]
enum State {
    Show,
    SelectToEdit,
    Edit(Field),
}

#[derive(Debug)]
pub enum ChaosEvent {
    TUIEvent(CEvent),
    Error(Result<()>),
    ChangeLeftTop(i32, i32),
    ChangeRightBottom(i32, i32),
    DataResponse(helper::ResponseFromNetwork),
    ChangeWindowSize(WindowRect),
}

use helper;
pub fn ui_loop<T: backend::Backend>(
    terminal: &mut Terminal<T>,
    mut save_data: SaveData,
    loop_proxy: crate::EventLoopProxy<ChaosEvent>,
    event_send: std::sync::mpsc::Sender<ChaosEvent>,
    event_recv: std::sync::mpsc::Receiver<ChaosEvent>,
) -> Result<()> {
    lazy_static! {
        static ref CLIPBOARD: Mutex<clipboard::ClipboardContext> =
            Mutex::new(clipboard::ClipboardProvider::new().unwrap());
    }

    loop_proxy.send_event(ChaosEvent::ChangeWindowSize(save_data.window_size.unwrap_or_default())).unwrap();

    std::thread::spawn(move || {
        while let Ok(e) = event::read() {
            if let Err(_) = event_send.send(ChaosEvent::TUIEvent(e)) {
                break;
            }
        }
    });

    let league_data = LEAGUE_DATA.as_ref().map_err(|e| anyhow!(e))?;

    let save_name = dirs::home_dir()
        .ok_or(anyhow!("Can't get home directory path"))?
        .join(SAVE_FILE_NAME);
    let mut state = State::Show;
    let mut error = String::new();
    for ce in event_recv.iter() {
        match ce {
            ChaosEvent::TUIEvent(e) => match e {
                CEvent::Key(KeyEvent {
                    code: KeyCode::Char(key),
                    modifiers,
                }) => match &mut state {
                    State::Show if key == 'e' => {
                        state = State::SelectToEdit;
                    }
                    State::Show if key == 'r' => {
                        helper::set_account(save_data.account_data.clone());
                        crate::IS_INITIALIZED.store(true, std::sync::atomic::Ordering::Release);
                        match helper::acquire_chaos_list(true) {
                            Ok(result) => {
                                loop_proxy.send_event(ChaosEvent::DataResponse(result))?;
                                error.clear();
                            }
                            Err(e) => error = e.to_string(),
                        }
                    }
                    State::Show if key == 's' => {
                        if let Err(e) = save_account_data(&save_name, &save_data) {
                            error = e.to_string();
                        } else {
                            error = "Save has been completed".to_string();
                        }
                    }
                    State::Show if key == 'q' => {
                        break;
                    }
                    State::SelectToEdit if key == '1' => {
                        state = State::Edit(Field::Account(String::new()));
                    }
                    State::SelectToEdit if key == '2' => {
                        state = State::Edit(Field::Cookie(String::new()));
                    }
                    State::SelectToEdit if key == '3' => {
                        state = State::Edit(Field::League(String::new()));
                    }
                    State::SelectToEdit if key == '4' => {
                        state = State::Edit(Field::TabIdx(None));
                    }
                    State::Edit(field) => {
                        if key == 'v' && modifiers == KeyModifiers::CONTROL {
                            if let Ok(clip) = CLIPBOARD.lock().unwrap().get_contents() {
                                for ch in clip.chars() {
                                    field.handle_input(ch);
                                }
                            }
                        } else {
                            field.handle_input(key);
                        }
                    }
                    _ => {}
                },
                CEvent::Key(KeyEvent {
                    code: KeyCode::Backspace,
                    modifiers,
                }) => {
                    if let State::Edit(f) = &mut state {
                        if modifiers == KeyModifiers::CONTROL {
                            f.erase_word();
                        } else {
                            f.erase();
                        }
                    }
                }
                CEvent::Key(KeyEvent { code, .. }) => match state {
                    _ if code == KeyCode::Esc => {
                        state = State::Show;
                    }
                    State::Edit(f) if code == KeyCode::Enter => {
                        match f {
                            Field::Account(s) => {
                                save_data.account_data.account = s;
                            }
                            Field::Cookie(s) => {
                                save_data.account_data.cookie = s;
                            }
                            Field::League(s) => {
                                save_data.account_data.league = s;
                            }
                            Field::TabIdx(i) => {
                                if let Some(idx) = i {
                                    save_data.account_data.tab_idx = idx as usize;
                                }
                            }
                        }
                        state = State::Show;
                    }
                    _ => {}
                },
                _ => {}
            },
            ChaosEvent::Error(Err(err)) => {
                error = err.to_string();
            }
            ChaosEvent::ChangeLeftTop(x, y) => {
                let mut win_size = save_data.window_size.unwrap_or_default();
                win_size.left = x;
                win_size.top = y;
                save_data.window_size = Some(win_size);
            }
            ChaosEvent::ChangeRightBottom(x, y) => {
                let mut win_size = save_data.window_size.unwrap_or_default();
                win_size.right = x;
                win_size.bottom = y;
                save_data.window_size = Some(win_size);
            }
            _ => {}
        }
        terminal.draw(|mut f| {
            draw_ui(&mut f, &state, &save_data, league_data, &error);
        })?;
    }
    Ok(())
}

fn draw_ui<T: backend::Backend>(
    f: &mut Frame<T>,
    state: &State,
    save_data: &SaveData,
    league_data: &Vec<String>,
    error: &str,
) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(4),
            ]
            .as_ref(),
        )
        .split(f.size());
    let key_help = Paragraph::new(
        [Text::Raw(Cow::Borrowed(
            "E/e : edit info, R/r: run helper, S/s: save data, Enter: finish editing, ESC: cancel editing",
        ))]
        .iter(),
    )
    .block(Block::default().borders(Borders::ALL).title("Key"))
    .wrap(true);
    f.render_widget(key_help, layout[0]);

    let middle_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
        .split(layout[1]);
    let tab_idx_string = save_data.account_data.tab_idx.to_string();
    let data_texts: Vec<&str> = vec![
        save_data.account_data.account.as_str(),
        save_data.account_data.cookie.as_str(),
        save_data.account_data.league.as_str(),
        tab_idx_string.as_str(),
    ];
    let data_labels = ["Account", "Web Cookie", "League", "Tab index"];
    let para_text: Vec<_> = match state {
        State::Show => data_texts
            .iter()
            .enumerate()
            .map(|(i, text)| {
                Text::Raw(Cow::Owned(format!(
                    "{}. {}: {}\n",
                    i + 1,
                    data_labels[i],
                    text
                )))
            })
            .collect(),
        State::SelectToEdit => data_texts
            .iter()
            .enumerate()
            .map(|(i, text)| {
                vec![
                    Text::Styled((i + 1).to_string().into(), Style::new().fg(Color::Blue)),
                    Text::Raw(format!(". {}: {}\n", data_labels[i], text).into()),
                ]
            })
            .flatten()
            .collect(),
        State::Edit(field) => {
            let edited: (usize, String);
            match field {
                Field::Account(val) => edited = (0, val.clone()),
                Field::Cookie(val) => edited = (1, val.clone()),
                Field::League(val) => edited = (2, val.clone()),
                Field::TabIdx(val) => edited = (3, val.map(|v| v.to_string()).unwrap_or_default()),
            }
            data_texts
                .iter()
                .enumerate()
                .map(|(i, text)| {
                    if edited.0 == i {
                        Text::Raw(Cow::Owned(format!("^{}: {}\n", data_labels[i], edited.1)))
                    } else {
                        Text::Styled(
                            Cow::Owned(format!("{}: {}\n", data_labels[i], text)),
                            Style::new().fg(Color::Black),
                        )
                    }
                })
                .collect()
        }
    };
    let para = Paragraph::new(para_text.iter())
        .block(Block::default().borders(Borders::ALL).title("Account"))
        .wrap(true);
    f.render_widget(para, middle_layout[0]);
    let leagues = league_data
        .iter()
        .map(|v| Text::Raw((v.clone() + "\n").into()))
        .collect::<Vec<_>>();
    let para = Paragraph::new(leagues.iter())
        .block(Block::default().borders(Borders::ALL).title("Leagues"))
        .wrap(true);
    f.render_widget(para, middle_layout[1]);
    let error_text = [Text::Raw(Cow::Borrowed(error))];
    let error_para = Paragraph::new(error_text.iter())
        .block(Block::default().borders(Borders::ALL).title("Info"))
        .wrap(true);
    f.render_widget(error_para, layout[2]);
}
