use std::{collections::BTreeMap, rc::Rc, sync::Arc};

use floem::{
    IntoView, View,
    action::{add_overlay, debounce_action, remove_overlay},
    context::LayoutChanged,
    event::listener as event,
    kurbo::Vec2,
    peniko::kurbo::{Point, Rect, Size},
    reactive::{
        Effect, Memo, ReadSignal, RwSignal, Scope, SignalGet, SignalRead,
        SignalUpdate, SignalWith,
    },
    style::CursorStyle,
    text::{Attrs, AttrsList, FamilyOwned, TextLayout},
    unit::DurationUnitExt,
    views::{
        Container, Decorators, Empty, Label, Scroll, Stack, TextInput,
        VirtualVector, dyn_stack,
        scroll::{PropagatePointerWheel, ScrollChanged},
        svg, virtual_stack,
    },
};
use indexmap::IndexMap;
use inflector::Inflector;
use lapce_rpc::plugin::VoltID;
use serde::Serialize;

use crate::{
    config::{
        DropdownInfo, LapceConfig, color::LapceColor,
        color_theme::resolve_text_input_style, core::CoreConfig,
        editor::EditorConfig, icon::LapceIcons, terminal::TerminalConfig,
        ui::UIConfig,
    },
    plugin::InstalledVoltData,
    window_tab::CommonData,
};

#[derive(Debug, Clone, Default)]
pub enum SettingsValue {
    Float(f64),
    Integer(i64),
    String(String),
    Bool(bool),
    Dropdown(DropdownInfo),
    #[default]
    Empty,
}

impl From<serde_json::Value> for SettingsValue {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Number(n) => {
                if n.is_f64() {
                    SettingsValue::Float(n.as_f64().unwrap())
                } else {
                    SettingsValue::Integer(n.as_i64().unwrap())
                }
            }
            serde_json::Value::String(s) => SettingsValue::String(s),
            serde_json::Value::Bool(b) => SettingsValue::Bool(b),
            _ => SettingsValue::Empty,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct SettingsItem {
    kind: String,
    name: String,
    field: String,
    description: String,
    filter_text: String,
    value: SettingsValue,
    // TODO: Why using serde_json if values are stored in toml?..
    serde_value: serde_json::Value,
    pos: RwSignal<Point>,
    size: RwSignal<Size>,
    // this is only the header that give an visual separation between different type of settings
    header: bool,
}

#[derive(Clone, Debug)]
struct SettingsData {
    items: RwSignal<imbl::Vector<SettingsItem>>,
    kinds: RwSignal<imbl::Vector<(String, RwSignal<Point>)>>,
    plugin_items: RwSignal<imbl::Vector<SettingsItem>>,
    plugin_kinds: RwSignal<imbl::Vector<(String, RwSignal<Point>)>>,
    filtered_items: RwSignal<imbl::Vector<SettingsItem>>,
    common: Rc<CommonData>,
}

impl SettingsData {
    pub fn new(
        cx: Scope,
        installed_plugin: RwSignal<IndexMap<VoltID, InstalledVoltData>>,
        common: Rc<CommonData>,
    ) -> Self {
        fn into_settings_map(
            data: &impl Serialize,
        ) -> serde_json::Map<String, serde_json::Value> {
            match serde_json::to_value(data).unwrap() {
                serde_json::Value::Object(h) => h,
                _ => serde_json::Map::default(),
            }
        }

        let config = common.config;
        let plugin_items = cx.create_rw_signal(imbl::Vector::new());
        let plugin_kinds = cx.create_rw_signal(imbl::Vector::new());
        let filtered_items = cx.create_rw_signal(imbl::Vector::new());
        let items = cx.create_rw_signal(imbl::Vector::new());
        let kinds = cx.create_rw_signal(imbl::Vector::new());
        cx.create_effect(move |_| {
            let config = config.get();

            let mut data_items = imbl::Vector::new();
            let mut data_kinds = imbl::Vector::new();
            let mut item_height_accum = 0.0;
            for (kind, fields, descs, mut settings_map) in [
                (
                    "Core",
                    &CoreConfig::FIELDS[..],
                    &CoreConfig::DESCS[..],
                    into_settings_map(&config.core),
                ),
                (
                    "Editor",
                    &EditorConfig::FIELDS[..],
                    &EditorConfig::DESCS[..],
                    into_settings_map(&config.editor),
                ),
                (
                    "UI",
                    &UIConfig::FIELDS[..],
                    &UIConfig::DESCS[..],
                    into_settings_map(&config.ui),
                ),
                (
                    "Terminal",
                    &TerminalConfig::FIELDS[..],
                    &TerminalConfig::DESCS[..],
                    into_settings_map(&config.terminal),
                ),
            ] {
                cx.enter(|| {
                    let pos = RwSignal::new(Point::new(0.0, item_height_accum));

                    data_items.push_back(SettingsItem {
                        kind: kind.to_string(),
                        pos,
                        header: true,
                        ..Default::default()
                    });
                    data_kinds.push_back((kind.to_string(), pos));
                });
                for (name, desc) in fields.iter().zip(descs.iter()) {
                    let field = name.replace('_', "-");

                    let (value, serde_value) = if let Some(dropdown) =
                        config.get_dropdown_info(&kind.to_lowercase(), &field)
                    {
                        let index = dropdown.active_index;
                        (
                            SettingsValue::Dropdown(dropdown),
                            serde_json::Value::Number(index.into()),
                        )
                    } else {
                        let value = settings_map.remove(&field).unwrap();
                        (SettingsValue::from(value.clone()), value)
                    };

                    let name = format!(
                        "{kind}: {}",
                        name.replace('_', " ").to_title_case()
                    );
                    let kind = kind.to_lowercase();
                    let filter_text = format!("{kind} {name} {desc}").to_lowercase();
                    let filter_text =
                        format!("{filter_text}{}", filter_text.replace(' ', ""));
                    cx.enter(|| {
                        data_items.push_back(SettingsItem {
                            kind,
                            name,
                            field,
                            filter_text,
                            description: desc.to_string(),
                            value,
                            serde_value,
                            ..Default::default()
                        });
                    });
                    item_height_accum += 50.0;
                }
            }

            filtered_items.set(data_items.clone());
            items.set(data_items);

            let plugins = installed_plugin.get();
            let mut setting_items = imbl::Vector::new();
            let mut plugin_kinds_tmp = imbl::Vector::new();
            for (_, volt) in plugins {
                let meta = volt.meta.get();
                let kind = meta.name;
                let plugin_config = config.plugins.get(&kind);
                if let Some(config) = meta.config {
                    cx.enter(|| {
                        let pos = RwSignal::new(Point::new(0.0, item_height_accum));

                        setting_items.push_back(SettingsItem {
                            kind: meta.display_name.clone(),
                            pos,
                            header: true,
                            ..Default::default()
                        });
                        plugin_kinds_tmp.push_back((meta.display_name.clone(), pos));
                    });

                    {
                        let mut local_items = Vec::new();
                        for (name, config) in config {
                            let field = name.clone();

                            let name = format!(
                                "{}: {}",
                                meta.display_name,
                                name.replace('_', " ").to_title_case()
                            );
                            let desc = config.description;
                            let filter_text =
                                format!("{kind} {name} {desc}").to_lowercase();
                            let filter_text = format!(
                                "{filter_text}{}",
                                filter_text.replace(' ', "")
                            );

                            let value = plugin_config
                                .and_then(|config| config.get(&field).cloned())
                                .unwrap_or(config.default)
                                .into();

                            cx.enter(|| {
                                local_items.push(SettingsItem {
                                    kind: kind.clone(),
                                    name,
                                    field,
                                    filter_text,
                                    description: desc.to_string(),
                                    value,
                                    ..Default::default()
                                });
                            });
                            item_height_accum += 50.0;
                        }
                        local_items.sort_by(|l, r| l.name.cmp(&r.name));
                        setting_items.extend(local_items);
                    }
                }
            }
            plugin_items.set(setting_items);
            plugin_kinds.set(plugin_kinds_tmp);
            kinds.set(data_kinds);
        });

        Self {
            filtered_items,
            plugin_items,
            plugin_kinds,
            items,
            kinds,
            common,
        }
    }
}

pub fn settings_view(
    installed_plugins: RwSignal<IndexMap<VoltID, InstalledVoltData>>,
    common: Rc<CommonData>,
) -> impl View {
    let config = common.config;

    let cx = Scope::current();
    let settings_data = SettingsData::new(cx, installed_plugins, common.clone());
    let view_settings_data = settings_data.clone();
    let plugin_kinds = settings_data.plugin_kinds;

    let filter_pattern: RwSignal<String> = Default::default();

    let items = settings_data.items;
    let kinds = settings_data.kinds;

    // TODO: This should be Memo.
    let filtered_items_signal = settings_data.filtered_items;
    Effect::new(move |_| {
        let plugin_items = settings_data.plugin_items.get();
        let mut items = items.get();
        let pattern = filter_pattern.get();
        if pattern.is_empty() {
            items.extend(plugin_items);
            filtered_items_signal.set(items);
            return;
        }

        let mut filtered_items = imbl::Vector::new();
        for item in &items {
            if item.header || item.filter_text.contains(&pattern) {
                filtered_items.push_back(item.clone());
            }
        }
        for item in plugin_items {
            if item.header || item.filter_text.contains(&pattern) {
                filtered_items.push_back(item);
            }
        }
        filtered_items_signal.set(filtered_items);
    });

    let ensure_visible = RwSignal::new(Rect::ZERO);
    let settings_content_size = RwSignal::new(Size::ZERO);
    let scroll_pos = RwSignal::new(Vec2::ZERO);

    let current_kind = {
        Memo::new(move |_| {
            let scroll_pos = scroll_pos.get();
            let scroll_y = scroll_pos.y + 30.0;

            let plugin_kinds = plugin_kinds.get_untracked();
            for (kind, pos) in plugin_kinds.iter().rev() {
                if pos.get_untracked().y < scroll_y {
                    return kind.to_string();
                }
            }

            let kinds = kinds.get();
            for (kind, pos) in kinds.iter().rev() {
                if pos.get_untracked().y < scroll_y {
                    return kind.to_string();
                }
            }

            kinds.get(0).unwrap().0.to_string()
        })
    };

    let switcher_item = move |k: String,
                              pos: Box<dyn Fn() -> Option<RwSignal<Point>>>,
                              margin: f32| {
        let kind = k.clone();
        Container::new(
            Label::new(k.clone())
                .style(move |s| s.text_ellipsis().padding_left(margin)),
        )
        .on_event_stop(event::Click, move |_, _| {
            if let Some(pos) = pos() {
                ensure_visible.set(
                    settings_content_size
                        .get_untracked()
                        .to_rect()
                        .with_origin(pos.get_untracked()),
                );
            }
        })
        .style(move |s| {
            let config = config.get();
            s.padding_horiz(20.0)
                .width_pct(100.0)
                .apply_if(kind == current_kind.get(), |s| {
                    s.background(config.color(LapceColor::PANEL_CURRENT_BACKGROUND))
                })
                .hover(|s| {
                    s.cursor(CursorStyle::Pointer).background(
                        config.color(LapceColor::PANEL_HOVERED_BACKGROUND),
                    )
                })
                .active(|s| {
                    s.background(
                        config.color(LapceColor::PANEL_HOVERED_ACTIVE_BACKGROUND),
                    )
                })
        })
    };

    let switcher = || {
        Stack::new((
            dyn_stack(
                move || kinds.get().clone(),
                |(k, _)| k.clone(),
                move |(k, pos)| switcher_item(k, Box::new(move || Some(pos)), 0.0),
            )
            .style(|s| s.flex_col().width_pct(100.0)),
            Stack::new((
                switcher_item(
                    "Plugin Settings".to_string(),
                    Box::new(move || {
                        plugin_kinds
                            .with_untracked(|k| k.get(0).map(|(_, pos)| *pos))
                    }),
                    0.0,
                ),
                dyn_stack(
                    move || plugin_kinds.get(),
                    |(k, _)| k.clone(),
                    move |(k, pos)| {
                        switcher_item(k, Box::new(move || Some(pos)), 10.0)
                    },
                )
                .style(|s| s.flex_col().width_pct(100.0)),
            ))
            .style(move |s| {
                s.width_pct(100.0)
                    .flex_col()
                    .apply_if(plugin_kinds.with(|k| k.is_empty()), |s| s.hide())
            }),
        ))
        .style(move |s| {
            s.width_pct(100.0)
                .flex_col()
                .line_height(1.8)
                .font_size(config.get().ui.font_size() as f32 + 1.0)
        })
    };

    Stack::new((
        Container::new({
            Scroll::new({
                Container::new(switcher())
                    .style(|s| s.padding_vert(20.0).width_pct(100.0))
            })
            .style(|s| s.absolute().size_pct(100.0, 100.0))
        })
        .style(move |s| {
            s.height_pct(100.0)
                .width(200.0)
                .border_right(1.0)
                .border_color(config.get().color(LapceColor::LAPCE_BORDER))
        }),
        Stack::new((
            Container::new({
                TextInput::new(filter_pattern)
                    .placeholder("Search Settings")
                    .style(move |_| {
                        resolve_text_input_style(config.get())
                            .width_pct(100.0)
                            .keyboard_navigable()
                    })
                    .request_focus(|| {})
            })
            .style(|s| s.padding_horiz(50.0).padding_vert(20.0)),
            Container::new({
                Scroll::new({
                    dyn_stack(
                        move || filtered_items_signal.get(),
                        |item| {
                            (
                                item.kind.clone(),
                                item.name.clone(),
                                item.serde_value.clone(),
                            )
                        },
                        move |item| {
                            settings_item_view(view_settings_data.clone(), item)
                        },
                    )
                    .style(|s| {
                        s.flex_col()
                            .padding_horiz(50.0)
                            .min_width_pct(100.0)
                            .max_width(400.0)
                    })
                })
                .on_event_stop(ScrollChanged::listener(), move |_, state| {
                    scroll_pos.set(state.offset);
                })
                .ensure_visible(move || ensure_visible.get())
                .on_event_stop(
                    LayoutChanged::listener(),
                    move |_, LayoutChanged { new_box, .. }| {
                        settings_content_size.set(new_box.size());
                    },
                )
                .style(|s| s.absolute().size_pct(100.0, 100.0))
            })
            .style(|s| s.size_pct(100.0, 100.0)),
        ))
        .style(|s| s.flex_col().size_pct(100.0, 100.0)),
    ))
    .style(|s| s.absolute().size_pct(100.0, 100.0))
    .debug_name("Settings")
}

fn settings_item_view(
    settings_data: SettingsData,
    item: SettingsItem,
) -> impl View + use<> {
    let config = settings_data.common.config;

    let is_ticked = if let SettingsValue::Bool(is_ticked) = &item.value {
        Some(*is_ticked)
    } else {
        None
    };

    let text_value = match &item.value {
        SettingsValue::Float(n) => Some(RwSignal::new(n.to_string())),
        SettingsValue::Integer(n) => Some(RwSignal::new(n.to_string())),
        SettingsValue::String(s) => Some(RwSignal::new(s.to_string())),
        SettingsValue::Bool(_) => None,
        SettingsValue::Dropdown(_) => None,
        SettingsValue::Empty => None,
    };

    let view = {
        if let Some(text_value) = text_value {
            debounce_action(text_value, 500.millis(), {
                // TODO: item_value is needed only for conversion to know the type, we don't need to capture actual value
                // which is stored in the enum variant(especially string).
                let item_value = item.serde_value;
                let kind = item.kind.clone();
                let field = item.field.clone();

                // NOTE: Settings are applied automatically but maybe it's a good idea to either
                // provide explicit apply / discard or at least a way to restore setting to previous value
                // and remove completely, resetting to default.
                // Another QoL improvement is to add an associated text signal - validation error
                // which is bound to label which shown only when non-empty label (maybe via dyn_contaier)
                // This function should just set parse error.
                // Or validation can run on each change immediately and produce signal <Value>, which
                // is comsumed in debounce_action without extra conversion.
                move || {
                    let value: Option<toml_edit::Value> = match &item_value {
                        serde_json::Value::Number(n) if n.is_f64() => {
                            text_value.read().parse::<f64>().map(Into::into).ok()
                        }
                        serde_json::Value::Number(_) => {
                            text_value.read().parse::<i64>().map(Into::into).ok()
                        }
                        _ => Some(text_value.get().into()),
                    };

                    if let Some(value) = value {
                        LapceConfig::update_file(&kind, &field, value);
                    }
                }
            });
            TextInput::new(text_value)
                .style(move |_| {
                    resolve_text_input_style(config.get())
                        .width(300.0)
                        .keyboard_navigable()
                })
                .into_any()
        } else if let SettingsValue::Dropdown(dropdown) = &item.value {
            let expanded = RwSignal::new(false);
            let current_value = dropdown
                .items
                .get(dropdown.active_index)
                .or_else(|| dropdown.items.last())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let current_value = RwSignal::new(current_value);

            dropdown_view(
                &item,
                current_value,
                dropdown,
                expanded,
                settings_data.common.window_common.size,
                config,
            )
            .into_any()
        } else if item.header {
            Label::new(item.kind.clone())
                .style(move |s| {
                    let config = config.get();
                    s.line_height(2.0)
                        .font_bold()
                        .width_pct(100.0)
                        .padding_horiz(10.0)
                        .font_size(config.ui.font_size() as f32 + 2.0)
                        .background(config.color(LapceColor::PANEL_BACKGROUND))
                })
                .into_any()
        } else {
            Empty::new().into_any()
        }
    };

    Stack::new((
        Label::new(item.name.clone()).style(move |s| {
            s.font_bold()
                .text_ellipsis()
                .min_width(0.0)
                .max_width_pct(100.0)
                .line_height(1.8)
                .font_size(config.get().ui.font_size() as f32 + 1.0)
        }),
        Stack::new((
            Label::new(item.description.clone()).style(move |s| {
                s.min_width(0.0)
                    .max_width_pct(100.0)
                    .line_height(1.8)
                    .apply_if(is_ticked.is_some(), |s| {
                        s.margin_left(config.get().ui.font_size() as f32 + 8.0)
                    })
                    .apply_if(item.header, |s| s.hide())
            }),
            if let Some(is_ticked) = is_ticked {
                let checked = RwSignal::new(is_ticked);

                let kind = item.kind.clone();
                let field = item.field.clone();
                Effect::new(move |last| {
                    let checked = checked.get();
                    if last.is_none() {
                        return;
                    }
                    if let Ok(value) = serde::Serialize::serialize(
                        &checked,
                        toml_edit::ser::ValueSerializer::new(),
                    ) {
                        LapceConfig::update_file(&kind, &field, value);
                    }
                });

                Container::new(
                    Stack::new((
                        checkbox(move || checked.get(), config),
                        Label::new(" ").style(|s| s.line_height(1.8)),
                    ))
                    .style(|s| s.items_center()),
                )
                .on_event_stop(event::Click, move |_, _| {
                    checked.update(|checked| {
                        *checked = !*checked;
                    });
                })
                .style(|s| {
                    s.absolute()
                        .cursor(CursorStyle::Pointer)
                        .size_pct(100.0, 100.0)
                        .items_start()
                })
            } else {
                Container::new(Empty::new()).style(|s| s.hide())
            },
        )),
        view.style(move |s| s.apply_if(!item.header, |s| s.margin_top(6.0))),
    ))
    .on_event_stop(
        LayoutChanged::listener(),
        move |_, LayoutChanged { new_box, .. }| {
            if item.header {
                item.pos.set(new_box.origin());
            }
            let old_size = item.size.get_untracked();
            let new_size = new_box.size();
            if old_size != new_size {
                item.size.set(new_size);
            }
        },
    )
    .style(|s| {
        s.flex_col()
            .padding_vert(10.0)
            .min_width_pct(100.0)
            .max_width(300.0)
    })
}

pub fn checkbox(
    checked: impl Fn() -> bool + 'static,
    config: ReadSignal<Arc<LapceConfig>>,
) -> impl View {
    const CHECKBOX_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -2 16 16"><polygon points="5.19,11.83 0.18,7.44 1.82,5.56 4.81,8.17 10,1.25 12,2.75" /></svg>"#;
    let svg_str = move || if checked() { CHECKBOX_SVG } else { "" }.to_string();

    svg(svg_str).style(move |s| {
        let config = config.get();
        let size = config.ui.font_size() as f32;
        let color = config.color(LapceColor::EDITOR_FOREGROUND);

        s.min_width(size)
            .size(size, size)
            .color(color)
            .border_color(color)
            .border(1.)
            .border_radius(2.)
    })
}

struct BTreeMapVirtualList(BTreeMap<String, String>);

impl VirtualVector<(String, String)> for BTreeMapVirtualList {
    fn total_len(&self) -> usize {
        self.0.len()
    }

    fn slice(
        &self,
        range: std::ops::Range<usize>,
    ) -> impl Iterator<Item = (String, String)> {
        Box::new(
            self.0
                .iter()
                .enumerate()
                .filter_map(|(index, (k, v))| {
                    if range.contains(&index) {
                        Some((k.to_string(), v.to_string()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .into_iter(),
        )
    }
}

fn color_section_list(
    kind: &str,
    header: &str,
    list: impl Fn() -> BTreeMap<String, String> + 'static,
    max_width: Memo<f64>,
    text_height: Memo<f64>,
    common: Rc<CommonData>,
) -> impl View {
    let config = common.config;

    let kind = kind.to_string();
    Stack::new((
        Label::new(header).style(|s| {
            s.margin_top(10)
                .margin_horiz(20)
                .font_bold()
                .line_height(2.0)
        }),
        virtual_stack(
            move || BTreeMapVirtualList(list()),
            move |(key, _)| key.to_owned(),
            move |(key, value)| {
                let value = RwSignal::new(value);

                debounce_action(value, 500.millis(), {
                    let kind = kind.clone();
                    let key = key.clone();

                    move || {
                        let config = config.get_untracked();
                        let default = match kind.as_str() {
                            "base" => config.default_color_theme().base.get(&key),
                            "ui" => config.default_color_theme().ui.get(&key),
                            "syntax" => {
                                config.default_color_theme().syntax.get(&key)
                            }
                            _ => None,
                        };

                        let parent = format!("color-theme.{kind}");

                        value.with(|text| match default {
                            Some(val) if val == text => {
                                LapceConfig::reset_setting(&parent, &key);
                            }
                            _ => {
                                LapceConfig::update_file(&parent, &key, text.into());
                            }
                        });
                    }
                });

                // TODO: Not sure what this effect did - it seems to react on changes to
                // config and replace value.
                // Howver updating string from editing feels strange - risk of infinite change loop
                // is probably low, but still effect will run several times until stabilizes...
                // {
                //     let kind = kind.clone();
                //     let key = key.clone();
                //     let doc = text_input_view.doc_signal();
                //     create_effect(move |_| {
                //         let doc = doc.get_untracked();
                //         let config = config.get();
                //         let current = doc.buffer.with_untracked(|b| b.to_string());

                //         let value = match kind.as_str() {
                //             "base" => config.color_theme.base.get(&key),
                //             "ui" => config.color_theme.ui.get(&key),
                //             "syntax" => config.color_theme.syntax.get(&key),
                //             _ => None,
                //         };

                //         if let Some(value) = value {
                //             if value != &current {
                //                 doc.reload(Rope::from(value.to_string()), true);
                //             }
                //         }
                //     });
                // }

                Stack::new((
                    Label::new(&key).style(move |s| {
                        s.width(max_width.get()).margin_left(20).margin_right(10)
                    }),
                    TextInput::new(value).style(move |_| {
                        resolve_text_input_style(config.get())
                            .width(150.0)
                            .margin_vert(6)
                            .keyboard_navigable()
                    }),
                    Empty::new().style({
                        let kind = kind.clone();
                        let key = key.clone();
                        move |s| {
                            let size = text_height.get() + 12.0;
                            let config = config.get();
                            let color = match kind.as_str() {
                                "base" => config.color.base.get(&key),
                                "ui" => config.color.ui.get(&key).copied(),
                                "syntax" => config.color.syntax.get(&key).copied(),
                                _ => None,
                            };
                            s.border(1)
                                .border_radius(6)
                                .size(size, size)
                                .margin_left(10)
                                .border_color(config.color(LapceColor::LAPCE_BORDER))
                                .background(color.unwrap_or_else(|| {
                                    config.color(LapceColor::EDITOR_FOREGROUND)
                                }))
                        }
                    }),
                    {
                        Label::new("Reset")
                            .on_event_stop(event::Click, {
                                let kind = kind.clone();
                                let key = key.clone();
                                move |_, _| {
                                    LapceConfig::reset_setting(
                                        &format!("color-theme.{kind}"),
                                        &key,
                                    );
                                }
                            })
                            .style({
                                let kind = kind.clone();
                                move |s| {
                                    let config = config.get();

                                    let same = 'same: {
                                        let theme = match kind.as_str() {
                                            "base" => {
                                                &config.default_color_theme().base.0
                                            }
                                            "ui" => &config.default_color_theme().ui,
                                            "syntax" => {
                                                &config.default_color_theme().syntax
                                            }

                                            _ => break 'same false,
                                        };
                                        theme
                                            .get(&key)
                                            .is_some_and(|val| *val == *value.read())
                                    };

                                    s.margin_left(10)
                                        .padding(6)
                                        .cursor(CursorStyle::Pointer)
                                        .border(1)
                                        .border_radius(6)
                                        .border_color(
                                            config.color(LapceColor::LAPCE_BORDER),
                                        )
                                        .apply_if(same, |s| s.hide())
                                        .active(|s| {
                                            s.background(
                                                config.color(
                                                    LapceColor::PANEL_BACKGROUND,
                                                ),
                                            )
                                        })
                                }
                            })
                    },
                ))
                .style(|s| s.items_center())
            },
        )
        .item_size_fixed(move || text_height.get() + 24.0)
        .style(|s| s.flex_col().padding_right(20)),
    ))
    .style(|s| s.flex_col())
}

pub fn theme_color_settings_view(common: Rc<CommonData>) -> impl View {
    let config = common.config;

    let text_height = Memo::new(move |_| {
        let mut text_layout = TextLayout::new();
        let config = config.get();
        let family: Vec<FamilyOwned> =
            FamilyOwned::parse_list(&config.ui.font_family).collect();
        let attrs = Attrs::new()
            .family(&family)
            .font_size(config.ui.font_size() as f32);
        let attrs_list = AttrsList::new(attrs);
        text_layout.set_text("W", attrs_list, None);
        text_layout.size().height
    });

    let max_width = Memo::new(move |_| {
        let mut text_layout = TextLayout::new();
        let config = config.get();
        let family: Vec<FamilyOwned> =
            FamilyOwned::parse_list(&config.ui.font_family).collect();
        let attrs = Attrs::new()
            .family(&family)
            .font_size(config.ui.font_size() as f32);
        let attrs_list = AttrsList::new(attrs);

        let mut max_width = 0.0;
        for key in config.color_theme.ui.keys() {
            text_layout.set_text(key, attrs_list.clone(), None);
            let width = text_layout.size().width;
            if width > max_width {
                max_width = width;
            }
        }
        for key in config.color_theme.syntax.keys() {
            text_layout.set_text(key, attrs_list.clone(), None);
            let width = text_layout.size().width;
            if width > max_width {
                max_width = width;
            }
        }
        max_width
    });

    let filter_pattern: RwSignal<String> = Default::default();

    Scroll::new(
        Stack::new((
            Container::new({
                TextInput::new(filter_pattern)
                    .placeholder("Search Settings")
                    .style(move |_| {
                        resolve_text_input_style(config.get())
                            .width_pct(100.0)
                            .keyboard_navigable()
                    })
                    .request_focus(|| {})
            })
            .style(|s| s.padding_vert(20.0).padding_horiz(20.0)),
            color_section_list(
                "base",
                "Base Colors",
                move || {
                    let config = config.get();
                    filter_pattern.with(|filter| {
                        config
                            .color_theme
                            .base
                            .0
                            .iter()
                            .filter_map(|x| {
                                if x.0.contains(filter) {
                                    Some((x.0.clone(), x.1.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect::<BTreeMap<String, String>>()
                    })
                },
                max_width,
                text_height,
                common.clone(),
            ),
            color_section_list(
                "syntax",
                "Syntax Colors",
                move || {
                    let config = config.get();
                    filter_pattern.with(|filter| {
                        config
                            .color_theme
                            .syntax
                            .iter()
                            .filter_map(|x| {
                                if x.0.contains(filter) {
                                    Some((x.0.clone(), x.1.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect::<BTreeMap<String, String>>()
                    })
                },
                max_width,
                text_height,
                common.clone(),
            ),
            color_section_list(
                "ui",
                "UI Colors",
                move || {
                    let config = config.get();
                    filter_pattern.with(|filter| {
                        config
                            .color_theme
                            .ui
                            .iter()
                            .filter_map(|x| {
                                if x.0.contains(filter) {
                                    Some((x.0.clone(), x.1.clone()))
                                } else {
                                    None
                                }
                            })
                            .collect::<BTreeMap<String, String>>()
                    })
                },
                max_width,
                text_height,
                common.clone(),
            ),
        ))
        .style(|s| s.flex_col()),
    )
    .style(|s| s.absolute().size_full())
    .debug_name("Theme Color Settings")
}

fn dropdown_view(
    item: &SettingsItem,
    current_value: RwSignal<String>,
    dropdown: &DropdownInfo,
    expanded: RwSignal<bool>,
    window_size: RwSignal<Size>,
    config: ReadSignal<Arc<LapceConfig>>,
) -> impl View + use<> {
    let window_origin = RwSignal::new(Point::ZERO);
    let size = RwSignal::new(Size::ZERO);
    let overlay_id = RwSignal::new(None);
    let dropdown_input_focus = RwSignal::new(false);
    let dropdown_scroll_focus = RwSignal::new(true);

    {
        let item = item.to_owned();
        let dropdown = dropdown.to_owned();
        Effect::new(move |_| {
            if expanded.get() {
                let item = item.clone();
                let dropdown = dropdown.clone();
                let id = add_overlay(dropdown_scroll(
                    &item.clone(),
                    current_value,
                    &dropdown.clone(),
                    expanded,
                    dropdown_scroll_focus,
                    dropdown_input_focus,
                    window_origin,
                    size,
                    window_size,
                    config,
                ));
                overlay_id.set(Some(id));
            } else if let Some(id) = overlay_id.get_untracked() {
                remove_overlay(id);
                overlay_id.set(None);
            }
        });
    }

    Stack::new((
        Label::derived(move || current_value).style(move |s| {
            s.text_ellipsis()
                .width_pct(100.0)
                .padding_horiz(10.0)
                .selectable(false)
        }),
        Container::new(
            svg(move || {
                if expanded.get() {
                    config.get().ui_svg(LapceIcons::CLOSE)
                } else {
                    config.get().ui_svg(LapceIcons::DROPDOWN_ARROW)
                }
            })
            .style(move |s| {
                let config = config.get();
                let size = config.ui.icon_size() as f32;
                s.size(size, size)
                    .color(config.color(LapceColor::LAPCE_ICON_ACTIVE))
            }),
        )
        .style(|s| s.padding_right(4.0)),
    ))
    .on_event_stop(event::Click, move |_, _| {
        expanded.update(|expanded| {
            *expanded = !*expanded;
        });
    })
    .on_event_stop(
        LayoutChanged::listener(),
        move |_,
              LayoutChanged {
                  new_box,
                  new_window_origin,
                  ..
              }| {
            window_origin.set(*new_window_origin);
            size.set(new_box.size());
            if expanded.get_untracked() {
                expanded.set(false);
            }
        },
    )
    .style(move |s| {
        s.items_center()
            .cursor(CursorStyle::Pointer)
            .border_color(config.get().color(LapceColor::LAPCE_BORDER))
            .border(1.0)
            .border_radius(6.0)
            .width(250.0)
            .line_height(1.8)
            .keyboard_navigable()
    })
    .on_event_stop(event::FocusGained, move |_, _| {
        dropdown_input_focus.set(true);
    })
    .on_event_stop(event::FocusLost, move |_, _| {
        dropdown_input_focus.set(false);
        if expanded.get_untracked() && !dropdown_scroll_focus.get_untracked() {
            expanded.set(false);
        }
    })
    .on_cleanup(move || {
        if let Some(id) = overlay_id.get_untracked() {
            remove_overlay(id);
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn dropdown_scroll(
    item: &SettingsItem,
    current_value: RwSignal<String>,
    dropdown: &DropdownInfo,
    expanded: RwSignal<bool>,
    dropdown_scroll_focus: RwSignal<bool>,
    dropdown_input_focus: RwSignal<bool>,
    window_origin: RwSignal<Point>,
    input_size: RwSignal<Size>,
    window_size: RwSignal<Size>,
    config: ReadSignal<Arc<LapceConfig>>,
) -> impl View + use<> {
    dropdown_scroll_focus.set(true);

    let kind = item.kind.clone();
    let field = item.field.clone();
    let view_fn = move |item_string: String| {
        let kind = kind.clone();
        let field = field.clone();

        Label::new(item_string.clone())
            .on_event_stop(event::Click, move |_, _| {
                current_value.set(item_string.clone());
                if let Ok(value) = serde::Serialize::serialize(
                    &item_string,
                    toml_edit::ser::ValueSerializer::new(),
                ) {
                    LapceConfig::update_file(&kind, &field, value);
                }
                expanded.set(false);
            })
            .style(move |s| {
                s.text_ellipsis().padding_horiz(10.0).hover(|s| {
                    s.cursor(CursorStyle::Pointer).background(
                        config.get().color(LapceColor::PANEL_HOVERED_BACKGROUND),
                    )
                })
            })
    };

    let items = dropdown.items.clone();

    let scroll_size = RwSignal::new(Size::ZERO);

    Scroll::new({
        dyn_stack(move || items.clone(), |item| item.to_string(), view_fn)
            .style(|s| s.flex_col().width_pct(100.0).cursor(CursorStyle::Pointer))
    })
    .style(move |s| {
        s.width_pct(100.0)
            .max_height(200.0)
            .set(PropagatePointerWheel, false)
            .keyboard_navigable()
    })
    .request_focus(|| {})
    .on_event_stop(event::FocusGained, move |_, _| {
        dropdown_scroll_focus.set(true);
    })
    .on_event_stop(event::FocusLost, move |_, _| {
        dropdown_scroll_focus.set(false);
        if expanded.get_untracked() && !dropdown_input_focus.get_untracked() {
            expanded.set(false);
        }
    })
    .on_event_stop(event::PointerMove, move |_, _| {})
    .on_event_stop(event::PointerDown, move |_, _| {})
    .on_event_stop(
        LayoutChanged::listener(),
        move |_, LayoutChanged { new_box, .. }| {
            scroll_size.set(new_box.size());
        },
    )
    .style(move |s| {
        let config = config.get();
        let window_origin = window_origin.get();
        let window_size = window_size.get();
        let input_size = input_size.get();
        let scroll_size = scroll_size.get();

        let x = if window_origin.x + scroll_size.width + 5.0 > window_size.width {
            window_size.width - scroll_size.width - 5.0
        } else {
            window_origin.x
        };

        let y = if window_origin.y + input_size.height + scroll_size.height + 5.0
            > window_size.height
        {
            window_origin.y - scroll_size.height + 1.0
        } else {
            window_origin.y + input_size.height - 1.0
        };

        s.width(250.0)
            .line_height(1.8)
            .font_size(config.ui.font_size() as f32)
            .font_family(config.ui.font_family.clone())
            .color(config.color(LapceColor::EDITOR_FOREGROUND))
            .background(config.color(LapceColor::EDITOR_BACKGROUND))
            .class(floem::views::scroll::Handle, |s| {
                s.background(config.color(LapceColor::LAPCE_SCROLL_BAR))
            })
            .border(1)
            .border_radius(6.0)
            .border_color(config.color(LapceColor::LAPCE_BORDER))
            .box_shadow_blur(3.0)
            .box_shadow_color(config.color(LapceColor::LAPCE_DROPDOWN_SHADOW))
            .inset_left(x)
            .inset_top(y)
    })
}
