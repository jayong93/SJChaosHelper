use anyhow::{anyhow, Context, Result};
use either::Either;
use lazy_static::lazy_static;
use regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fmt::{self, Formatter};
use std::iter::Iterator;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Mutex, RwLock,
};
use std::thread::spawn;

lazy_static! {
    static ref CLIENT: Client = Client::new();
    static ref ACCOUNT: RwLock<AccountData> = RwLock::new(AccountData {
        account: String::new(),
        cookie: String::new(),
        league: String::new(),
        tab_idx: 0,
    });
    static ref NET_THREAD_SENDER: Mutex<Option<mpsc::Sender<InternalMessage>>> = Mutex::new(None);
    static ref DEBUG_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static ref IS_QUAD_STASH: AtomicBool = AtomicBool::new(false);
}

#[derive(Default, Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountData {
    pub account: String,
    pub cookie: String,
    pub league: String,
    pub tab_idx: usize,
}

pub fn save_account_data(path: &std::path::Path, account: &AccountData) -> Result<()> {
    use serde_json::to_writer;
    use std::fs::OpenOptions;

    let out_file = OpenOptions::new().create(true).write(true).open(path)?;
    to_writer(out_file, account)?;

    Ok(())
}

pub fn load_account_data(path: &std::path::Path) -> Result<AccountData> {
    use serde_json::from_reader;
    use std::fs::OpenOptions;
    let out_file = OpenOptions::new().read(true).open(path)?;
    from_reader(out_file).map_err(|e| anyhow!(e))
}

#[derive(Deserialize)]
struct League {
    id: String,
}

#[derive(Deserialize)]
struct LeagueList(Vec<League>);

pub fn get_league_list() -> Result<Vec<String>> {
    CLIENT
        .get("http://api.pathofexile.com/leagues")
        .query(&[("compact", 1)])
        .send()
        .and_then(|mut res| {
            let val: LeagueList = res.json()?;
            Ok(val.0.into_iter().map(|league| league.id).collect())
        })
        .map_err(|e| anyhow!(e))
}

pub fn init_module() {
    let (sender, receiver) = mpsc::channel();
    spawn(network_thread_func(receiver));
    let mut g_sender = NET_THREAD_SENDER.lock().unwrap();
    *g_sender = Some(sender);
}

pub fn set_account(new_account: AccountData) {
    let mut g_account = ACCOUNT.write().unwrap();
    if *g_account == new_account {
        return;
    }
    *g_account = new_account;
}

/// (Chaos-able-items, Regal-able-items)
type ClassifiedRecipeLists = (Vec<Item>, Vec<Item>);
/// <ItemType, (Chaos-able-items, Regal-able-items)>
type ChaosRecipeSet = HashMap<ItemType, ClassifiedRecipeLists>;

#[derive(Clone)]
struct ChaosListGenerator<'a> {
    stash_items: HashMap<ItemType, (&'a [Item], &'a [Item])>,
}

impl<'a> ChaosListGenerator<'a> {
    fn new(map: &'a ChaosRecipeSet) -> Self {
        Self {
            stash_items: map
                .iter()
                .map(|(k, (c, r))| (*k, (c.as_slice(), r.as_slice())))
                .collect(),
        }
    }

    fn get_item_by_type(
        &mut self,
        i_type: ItemType,
        can_make_chaos: bool,
    ) -> Option<Either<&'a Item, &'a Item>> {
        self.stash_items
            .get_mut(&i_type)
            .and_then(|list_tuple| Self::get_item(list_tuple, can_make_chaos))
    }

    fn get_item(
        list_tuple: &mut (&'a [Item], &'a [Item]),
        can_make_chaos: bool,
    ) -> Option<Either<&'a Item, &'a Item>> {
        let chaos_list = list_tuple.0;
        let regal_list = list_tuple.1;

        match can_make_chaos {
            true => regal_list
                .split_first()
                .map(|(item, remains)| {
                    list_tuple.1 = remains;
                    Either::Right(item)
                })
                .or_else(|| {
                    chaos_list.split_first().map(|(item, remains)| {
                        list_tuple.0 = remains;
                        Either::Left(item)
                    })
                }),
            false => chaos_list
                .split_first()
                .map(|(item, remains)| {
                    list_tuple.0 = remains;
                    Either::Left(item)
                })
                .or_else(|| {
                    regal_list.split_first().map(|(item, remains)| {
                        list_tuple.1 = remains;
                        Either::Right(item)
                    })
                }),
        }
    }

    fn get_weapon_items(&mut self, can_make_chaos: bool) -> Option<Either<Vec<Item>, &'a Item>> {
        self.stash_items
            .get_mut(&ItemType::Weapon1HOrShield)
            .and_then(|list_tuple| match can_make_chaos {
                true => Self::get_item(list_tuple, can_make_chaos).and_then(|e| {
                    let mut vec = vec![e.into_inner().clone()];
                    Self::get_item(list_tuple, can_make_chaos).map(|e| {
                        vec.push(e.into_inner().clone());
                        vec
                    })
                }),
                false => Self::get_item(list_tuple, can_make_chaos).and_then(|e| {
                    e.either_with(
                        list_tuple,
                        |list_tuple, item| {
                            Self::get_item(list_tuple, true)
                                .map(|e| vec![e.into_inner().clone(), item.clone()])
                        },
                        |list_tuple, item| {
                            Self::get_item(list_tuple, false).and_then(|e| {
                                e.left().map(|item2| vec![item.clone(), item2.clone()])
                            })
                        },
                    )
                }),
            })
            .map(|items| Either::Left(items))
            .or_else(|| {
                self.stash_items
                    .get_mut(&ItemType::Weapon2H)
                    .and_then(|list_tuple| {
                        Self::get_item(list_tuple, can_make_chaos).and_then(|e| {
                            if can_make_chaos {
                                Some(e.into_inner())
                            } else {
                                e.left()
                            }
                        })
                    })
                    .map(|item| Either::Right(item))
            })
    }
}

impl<'a> Iterator for ChaosListGenerator<'a> {
    type Item = Vec<Item>;

    fn next(&mut self) -> Option<Self::Item> {
        // 무기가 아닌 것들을 모아서 하나씩 벡터에 넣는다.
        let recipe_without_weapons: [ItemType; 8] = [
            ItemType::Amulet,
            ItemType::Belt,
            ItemType::Body,
            ItemType::Boots,
            ItemType::Gloves,
            ItemType::Helmet,
            ItemType::Ring,
            ItemType::Ring,
        ];

        let mut can_make_chaos = false;
        let result_vec: Option<Self::Item> =
            recipe_without_weapons
                .iter()
                .cloned()
                .try_fold(vec![], |mut vec, i_type| {
                    let item: Option<Either<&'a Item, &'a Item>> =
                        self.get_item_by_type(i_type, can_make_chaos);
                    item.map(|e| {
                        vec.push(
                            e.right_or_else(|item| {
                                can_make_chaos = true;
                                item
                            })
                            .clone(),
                        );
                        vec
                    })
                });
        result_vec.and_then(|mut vec| {
            let weapon_result = self.get_weapon_items(can_make_chaos);
            weapon_result.map(|e| {
                e.either_with(
                    &mut vec,
                    |vec, mut items| {
                        // if w1h
                        vec.append(&mut items);
                    },
                    |vec, item| {
                        // if w2h
                        vec.push(item.clone())
                    },
                );
                vec
            })
        })
    }
}

fn network_thread_func(recv: mpsc::Receiver<InternalMessage>) -> impl FnOnce() -> () {
    move || {
        let (in_send, in_recv) = mpsc::sync_channel::<()>(1);
        let (data_send, data_recv) = mpsc::channel::<Result<ChaosRecipeSet>>();
        {
            spawn(move || {
                for _ in in_recv.iter() {
                    match get_stash_data_in() {
                        Ok(stash_data) => {
                            {
                                IS_QUAD_STASH.store(stash_data.quad_layout, Ordering::Relaxed);
                            }
                            let mut map: ChaosRecipeSet = HashMap::new();
                            for item in stash_data.items {
                                if item.ilvl < 60 || item.frame_type != 2 {
                                    continue;
                                }
                                let (chaos_list, regal_list) = map.entry(item.itype).or_default();
                                if item.ilvl < 75 {
                                    chaos_list.push(item);
                                } else {
                                    regal_list.push(item);
                                }
                            }
                            data_send.send(Ok(map)).unwrap();
                        }
                        Err(e) => data_send.send(Err(e)).unwrap(),
                    }
                }
            });
        }

        let mut map: ChaosRecipeSet = HashMap::new();
        let mut chaos_queue: VecDeque<Vec<Item>> = ChaosListGenerator::new(&map).collect();
        let mut total_count = chaos_queue.len();

        for msg in recv.iter() {
            let is_quad_stash = IS_QUAD_STASH.load(Ordering::Relaxed);
            match msg {
                InternalMessage::RequestChaosRecipe(sender) => match chaos_queue.pop_front() {
                    Some(chaos_list) => sender
                        .send(Ok(ResponseFromNetwork::ChaosRecipe((
                            chaos_list,
                            is_quad_stash,
                        ))))
                        .unwrap(),
                    None => sender
                        .send(Ok(ResponseFromNetwork::ChaosRecipe((
                            Vec::new(),
                            is_quad_stash,
                        ))))
                        .unwrap(),
                },
                InternalMessage::RequestStashStatus(sender) => {
                    in_send.try_send(()).ok();
                    let recv_result = data_recv.try_iter().last();
                    match recv_result {
                        Some(Ok(new_map)) => {
                            if new_map != map {
                                map = new_map;
                                chaos_queue = ChaosListGenerator::new(&map).collect();
                                total_count = chaos_queue.len();
                            }
                            sender
                                .send(Ok(ResponseFromNetwork::StashStatus((
                                    map.clone(),
                                    total_count,
                                ))))
                                .unwrap();
                        }
                        Some(Err(e)) => {
                            sender.send(Err(e)).unwrap();
                        }
                        None => {
                            sender
                                .send(Ok(ResponseFromNetwork::StashStatus((
                                    map.clone(),
                                    total_count,
                                ))))
                                .unwrap();
                        }
                    }
                }
            }
        }
    }
}

pub fn acquire_chaos_list(requre_whole: bool) -> Result<ResponseFromNetwork> {
    let (sender, receiver) = mpsc::channel();
    let g_sender = NET_THREAD_SENDER.lock().unwrap();
    g_sender
        .as_ref()
        .unwrap()
        .send(match requre_whole {
            true => InternalMessage::RequestStashStatus(sender),
            false => InternalMessage::RequestChaosRecipe(sender),
        })
        .map_err(|e| anyhow!("{}", e))?;
    let ret_val = match receiver.iter().last() {
        Some(val @ Ok(_)) => val,
        Some(val @ Err(_)) => val,
        _ => Err(anyhow::anyhow!("Network Thread Channel has send nothing")),
    };
    ret_val
}

#[derive(Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub w: usize,
    pub h: usize,
    pub x: usize,
    pub y: usize,
    ilvl: usize,
    #[serde(rename = "frameType")]
    frame_type: usize, // number 2 is unique
    #[serde(deserialize_with = "item_type_from_icon", rename = "icon")]
    itype: ItemType,
}

fn item_type_from_icon<'de, D>(d: D) -> Result<ItemType, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    let visitor = ItemTypeVisitor;
    d.deserialize_identifier(visitor)
}

#[derive(Deserialize, Debug)]
struct StashData {
    items: Vec<Item>,
    #[serde(default, rename = "quadLayout")]
    quad_layout: bool,
}

fn get_stash_data_in() -> Result<StashData> {
    let res;
    let account;
    {
        let g_account = ACCOUNT.read().unwrap();
        account = g_account.clone();
    }
    res = CLIENT
        .get("https://poe.game.daum.net/character-window/get-stash-items")
        .query(&[
            ("accountName", account.account.as_str()),
            ("realm", "pc"),
            ("league", account.league.as_str()),
        ])
        .query(&[("tabs", 0)])
        .query(&[("tabIndex", account.tab_idx)])
        .query(&[("public", false)])
        .header("Cookie", account.cookie)
        // .header("Host", "www.pathofexile.com")
        // .header("Connection", "Keep-Alive")
        .send()?;
    let status = res.status().as_u16();
    match res.error_for_status() {
        Ok(mut res) => res
            .json()
            .with_context(move || format!("status: {}\nheaders: {:?}", status, res.headers())),
        Err(e) => Err(anyhow!(e)),
    }
}

use strum_macros::*;
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, AsRefStr)]
pub enum ItemType {
    Weapon1HOrShield,
    Weapon2H,
    Body,
    Helmet,
    Boots,
    Gloves,
    Ring,
    Amulet,
    Belt,
    Useless,
}

struct ItemTypeVisitor;

impl<'de> serde::de::Visitor<'de> for ItemTypeVisitor {
    type Value = ItemType;

    fn expecting(&self, fomatter: &mut Formatter) -> fmt::Result {
        write!(fomatter, "a icon image url which contains item types")
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        use regex::Regex;
        use serde::de;
        lazy_static! {
            static ref RE: Regex = Regex::new(r"/2DItems/(.+?)/(.+?)(\.png|/)").unwrap();
        }
        let cap = RE.captures(s);
        if let Some(cap) = cap {
            match (
                cap.get(1).map(|m| m.as_str()),
                cap.get(2).map(|m| m.as_str()),
            ) {
                (Some("Armours"), Some("Boots")) => Ok(ItemType::Boots),
                (Some("Armours"), Some("Helmets")) => Ok(ItemType::Helmet),
                (Some("Armours"), Some("Gloves")) => Ok(ItemType::Gloves),
                (Some("Armours"), Some("BodyArmours")) => Ok(ItemType::Body),
                (Some("Armours"), Some("Shields")) => Ok(ItemType::Weapon1HOrShield),
                (Some("Weapons"), Some("OneHandWeapons")) => Ok(ItemType::Weapon1HOrShield),
                (Some("Weapons"), Some("TwoHandWeapons")) => Ok(ItemType::Weapon2H),
                (Some("Weapons"), Some("Bows")) => Ok(ItemType::Weapon2H),
                (Some("Amulets"), _) => Ok(ItemType::Amulet),
                (Some("Rings"), _) => Ok(ItemType::Ring),
                (Some("Belts"), _) => Ok(ItemType::Belt),
                (Some(_), Some(_)) => Ok(ItemType::Useless),
                _ => Err(de::Error::invalid_value(de::Unexpected::Str(s), &self)),
            }
        } else {
            Err(de::Error::invalid_value(de::Unexpected::Str(s), &self))
        }
    }
}

#[derive(Clone)]
enum InternalMessage {
    RequestChaosRecipe(mpsc::Sender<Result<ResponseFromNetwork>>),
    RequestStashStatus(mpsc::Sender<Result<ResponseFromNetwork>>),
}

#[derive(Clone, Debug)]
pub enum ResponseFromNetwork {
    /// items in a chaos recipe and whether it's quad stash
    ChaosRecipe((Vec<Item>, bool)),
    /// recipe set and total able chaos orbs
    StashStatus((ChaosRecipeSet, usize)),
}
