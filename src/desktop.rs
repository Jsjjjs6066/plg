use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex, RwLock},
    thread,
};

#[cfg(not(unix))]
use std::process::Command;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::{Config, load_config};
use chrono::TimeDelta;
use plg::{LogOptions, log, set_log_sink};
use qtrs::*;
use clap::CommandFactory;

pub struct State {
    update_option: u8,
    force_push: bool,
    url: String,
    remote: String,
    name: String,
    message: String,
    player: String,
    args: String,
    default_player: String,
    update_cooldown_m: i32,
    update_cooldown_h: i32,
    disable_update_on_play: bool,
    reset_default_player: bool,
    reset_update_cooldown: bool,
    reset_disable_update_on_play: bool,
    reinit: bool,
    shortcut_name: String,
    output_dir: String,
}

impl Default for State {
    fn default() -> Self {
        State {
            update_option: 0,
            force_push: false,
            url: String::new(),
            remote: String::new(),
            name: String::new(),
            message: String::new(),
            player: String::new(),
            args: String::new(),
            default_player: String::new(),
            update_cooldown_m: 0,
            update_cooldown_h: 0,
            disable_update_on_play: false,
            reset_default_player: false,
            reset_update_cooldown: false,
            reset_disable_update_on_play: false,
            reinit: false,
            shortcut_name: String::new(),
            output_dir: String::new(),
        }
    }
}

fn restart(code: i32) -> ! {
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            error_box(format!("Unable to restart: {error}"));
            std::process::exit(code);
        }
    };
    let arguments = std::env::args_os().skip(1);

    #[cfg(unix)]
    {
        let error = std::process::Command::new(executable)
            .args(arguments)
            .exec();
        error_box(format!("Unable to restart: {error}"));
        std::process::exit(code);
    }

    #[cfg(not(unix))]
    {
        if let Err(error) = Command::new(executable).args(arguments).spawn() {
            error_box(format!("Unable to restart: {error}"));
        }
        std::process::exit(code);
    }
}

fn update_options(state: &Arc<RwLock<State>>, font: &Font) -> ComboBox {
    let mut options = ComboBox::new()
        .items(&["Default update", "No update", "Force update"])
        .build();
    let state = Arc::clone(state);
    options.connect_current_index_changed(move |index| {
        state.write().unwrap().update_option = index as u8;
    });
    options.set_font(font);
    options
}

fn update_values(state: &Arc<RwLock<State>>) -> (bool, bool) {
    match state.read().unwrap().update_option {
        1 => (true, false),
        2 => (false, true),
        _ => (false, false),
    }
}

fn error_box(error: impl ToString) {
    MessageBox::new()
        .icon(CRITICAL)
        .text(error.to_string())
        .window_title("Error")
        .build()
        .exec();
}

fn append_output(output: &mut String, chunk: &str) {
    for character in chunk.chars() {
        if character == '\r' {
            if let Some(line_start) = output.rfind('\n') {
                output.truncate(line_start + 1);
            } else {
                output.clear();
            }
        } else {
            output.push(character);
        }
    }
}

fn run_command(command: crate::Cmds, lo: LogOptions, restart_after: bool) {
    thread::spawn(move || {
        if let Err(error) = crate::run(command, lo) {
            log!(lo, "Error: {error}");
        } else if restart_after {
            restart(0);
        }
    });
}

struct SharedTextBrowser(Rc<RefCell<TextBrowser>>);

impl AsWidget for SharedTextBrowser {
    fn widget_ptr(&self) -> *mut qtrs::ffi::QWidget {
        self.0.borrow().widget_ptr()
    }

    fn set_has_parent(&mut self) {
        self.0.borrow_mut().set_has_parent();
    }
}

struct SharedLineEdit(Rc<RefCell<LineEdit>>);

impl AsWidget for SharedLineEdit {
    fn widget_ptr(&self) -> *mut qtrs::ffi::QWidget {
        self.0.borrow().widget_ptr()
    }

    fn set_has_parent(&mut self) {
        self.0.borrow_mut().set_has_parent();
    }
}

struct SharedSpinBox(Rc<RefCell<SpinBox>>);

impl AsWidget for SharedSpinBox {
    fn widget_ptr(&self) -> *mut qtrs::ffi::QWidget {
        self.0.borrow().widget_ptr()
    }

    fn set_has_parent(&mut self) {
        self.0.borrow_mut().set_has_parent();
    }
}

struct SharedButton(Rc<RefCell<PushButton>>);

impl AsWidget for SharedButton {
    fn widget_ptr(&self) -> *mut qtrs::ffi::QWidget {
        self.0.borrow().widget_ptr()
    }

    fn set_has_parent(&mut self) {
        self.0.borrow_mut().set_has_parent();
    }
}

struct SharedComboBox(Rc<RefCell<ComboBox>>);

impl AsWidget for SharedComboBox {
    fn widget_ptr(&self) -> *mut qtrs::ffi::QWidget {
        self.0.borrow().widget_ptr()
    }

    fn set_has_parent(&mut self) {
        self.0.borrow_mut().set_has_parent();
    }
}

fn add_field(
    layout: &mut VBoxLayout,
    state: &Arc<RwLock<State>>,
    field: fn(&mut State) -> &mut String,
    label: &str,
    font: &Font,
) -> Rc<RefCell<LineEdit>> {
    let label_widget = Label::new(label).build();
    label_widget.set_font(font);
    layout.add_widget(Box::new(label_widget));

    let edit = Rc::new(RefCell::new(LineEdit::new("").build()));
    edit.borrow().set_font(font);
    let state_for_edit = Arc::clone(state);
    let edit_for_signal = Rc::downgrade(&edit);
    edit.borrow_mut().connect_return_pressed(move || {
        if let Some(edit) = edit_for_signal.upgrade() {
            edit.borrow_mut().refresh_text();
            field(&mut state_for_edit.write().unwrap())
                .clone_from(&edit.borrow().text().to_owned());
        }
    });
    layout.add_widget(Box::new(SharedLineEdit(Rc::clone(&edit))));
    edit
}

fn add_field_value(
    layout: &mut VBoxLayout,
    state: &Arc<RwLock<State>>,
    field: fn(&mut State) -> &mut String,
    label: &str,
    font: &Font,
    value: String,
) -> Rc<RefCell<LineEdit>> {
    let label_widget = Label::new(label).build();
    label_widget.set_font(font);
    layout.add_widget(Box::new(label_widget));

    let edit = Rc::new(RefCell::new(LineEdit::new("").build()));
    edit.borrow().set_font(font);
    edit.borrow_mut().set_text(value);
    let state_for_edit = Arc::clone(state);
    let edit_for_signal = Rc::downgrade(&edit);
    edit.borrow_mut().connect_return_pressed(move || {
        if let Some(edit) = edit_for_signal.upgrade() {
            edit.borrow_mut().refresh_text();
            field(&mut state_for_edit.write().unwrap())
                .clone_from(&edit.borrow().text().to_owned());
        }
    });
    layout.add_widget(Box::new(SharedLineEdit(Rc::clone(&edit))));
    edit
}

fn add_time_field_value(
    layout: &mut VBoxLayout,
    label: &str,
    font: &Font,
    value: (i32, i32),
) -> (Rc<RefCell<SpinBox>>, Rc<RefCell<SpinBox>>) {
    let label_widget = Label::new(label).build();
    label_widget.set_font(font);
    layout.add_widget(Box::new(label_widget));

    let mut vb = GridLayout::new();

    let hours = Rc::new(RefCell::new(SpinBox::new().range(0, 23).suffix(" hours").build()));
    hours.borrow().set_font(font);
    hours.borrow_mut().set_value(value.0);
    vb.add_widget(Box::new(SharedSpinBox(Rc::clone(&hours))), 0, 0, 1, 1);
    
    let minutes = Rc::new(RefCell::new(SpinBox::new().range(0, 59).suffix(" minutes").build()));
    minutes.borrow().set_font(font);
    minutes.borrow_mut().set_value(value.1);
    vb.add_widget(Box::new(SharedSpinBox(Rc::clone(&minutes))), 0, 1, 1, 1);
    
    let mut wid = Widget::new().build();
    wid.set_layout(&vb);
    wid.show();
    std::mem::forget(vb);
    
    layout.add_widget(Box::new(wid));
    (hours, minutes)
}

fn sync_field(
    edit: &Rc<RefCell<LineEdit>>,
    state: &Arc<RwLock<State>>,
    field: fn(&mut State) -> &mut String,
) -> String {
    edit.borrow_mut().refresh_text();
    let value = edit.borrow().text().to_owned();
    field(&mut state.write().unwrap()).clone_from(&value);
    value
}

fn sync_field_number(
    edit: &Rc<RefCell<SpinBox>>,
    state: &Arc<RwLock<State>>,
    field: fn(&mut State) -> &mut i32,
) -> i32 {
    let value = edit.borrow().value().to_owned();
    field(&mut state.write().unwrap()).clone_from(&value);
    value
}

fn page() -> (Widget, VBoxLayout) {
    let widget = Widget::new().build();
    let layout = VBoxLayout::new();
    (widget, layout)
}

fn add_update_controls(layout: &mut VBoxLayout, state: &Arc<RwLock<State>>, font: &Font) {
    let label = Label::new("Update options").build();
    label.set_font(font);
    layout.add_widget(Box::new(label));
    layout.add_widget(Box::new(update_options(state, font)));
}

pub fn run_ui(c: &Config, state: &Arc<RwLock<State>>, font: &Font, lo: LogOptions, selector: Rc<RefCell<ComboBox>>) -> Box<dyn AsWidget> {
    let (mut widget, mut layout) = page();
    add_update_controls(&mut layout, state, font);

    let mut run_in_w = Widget::new().build();
    let mut run_in_l = GridLayout::new();

    let now = chrono::Utc::now().naive_utc();
    if (now - TimeDelta::minutes(c.update_cooldown_m.unwrap_or_default() as i64)) < c.last_updated.unwrap_or_default() {
                let l = Label::new(format!(
                    "By default skipping update because of the update cooldown of {} minutes. Use the update options to override this behavior. Next update will be available in {} minutes.",
                c.update_cooldown_m.unwrap_or_default(),
                (c.last_updated.unwrap_or_default() + TimeDelta::minutes(c.update_cooldown_m.unwrap_or_default() as i64) - now).num_minutes()
                )).build();
            l.set_font(&Font::new().point_size(14).build());
            l.set_size_policy(1, 5);
            let mut notice = Widget::new().build();
            notice.set_style_sheet("QLabel { qproperty-wordWrap: true; }");
            let mut notice_layout = VBoxLayout::new();
            notice_layout.add_widget(Box::new(l));
            notice.set_layout(&notice_layout);
            std::mem::forget(notice_layout);
            layout.add_widget(Box::new(notice));
        }

    let default_player = if let Some(player) = c.default_player.clone() {
        PushButton::new(format!("Run in {player}"))
            .on_clicked(move || {
                run_command(crate::Cmds::Play {
                    no_update: false,
                    force_update: false,
                    player: None,
                    args: Vec::new(),
                }, lo, false)
            })
            .build()
    }
    else {
        PushButton::new("Configure default player")
            .on_clicked(move || {
                selector.borrow_mut().set_current_index(6);
            })
            .build()
    };
    default_player.set_font(&Font::new().point_size(22).bold(true).build());
    default_player.set_focus();

    layout.add_widget(Box::new(default_player));

    let player_field = {
        let edit = Rc::new(RefCell::new(LineEdit::new("").build()));
        edit.borrow().set_font(font);
        let state_for_edit = Arc::clone(state);
        let edit_for_signal = Rc::downgrade(&edit);
        edit.borrow_mut().connect_return_pressed(move || {
            if let Some(edit) = edit_for_signal.upgrade() {
                edit.borrow_mut().refresh_text();
                state_for_edit.write().unwrap().player
                    .clone_from(&edit.borrow().text().to_owned());
            }
        });
        run_in_l.add_widget(Box::new(SharedLineEdit(Rc::clone(&edit))), 0, 1, 1, 1);
        edit
    };

    run_in_w.set_layout(&run_in_l);
    layout.add_widget(Box::new(run_in_w));

    let args_field = add_field(
        &mut layout,
        state,
        |state| &mut state.args,
        "Player arguments (optional)",
        font,
    );
    {
        let state_for_play = Arc::clone(state);
        let player_for_play = Rc::clone(&player_field);
        let args_for_play = Rc::clone(&args_field);
        let play_in = PushButton::new("Play in...")
            .on_clicked(move || {
                let (no_update, force_update) = update_values(&state_for_play);
                let player = sync_field(&player_for_play, &state_for_play, |state| &mut state.player);
                let args = sync_field(&args_for_play, &state_for_play, |state| &mut state.args);
                run_command(crate::Cmds::Play {
                    no_update,
                    force_update,
                    player: (!player.is_empty()).then_some(player),
                    args: args.split_whitespace().map(str::to_owned).collect(),
                }, lo, false);
            })
            .build();
        play_in.set_font(font);
        run_in_l.add_widget(Box::new(play_in), 0, 0, 1, 1);
    }
    
    std::mem::forget(run_in_l);

    layout.add_spacer(Spacer::vertical_expanding());
    widget.set_layout(&layout);
    std::mem::forget(layout);
    Box::new(widget)
}

fn other(
    state: &Arc<RwLock<State>>,
    title: &str,
    command: crate::Cmds,
    font: &Font,
    lo: LogOptions,
) -> Box<dyn AsWidget> {
    let config = load_config().unwrap_or_default();
    let (mut widget, mut layout) = page();
    if matches!(
        &command,
        crate::Cmds::Add { .. } | crate::Cmds::Update { .. } | crate::Cmds::Push { .. }
    ) {
        add_update_controls(&mut layout, state, font);
    }
    let url_field = if matches!(
        &command,
        crate::Cmds::Add { .. } | crate::Cmds::Download { .. }
    ) {
        Some(add_field(
            &mut layout,
            state,
            |state| &mut state.url,
            "URL (required)",
            font,
        ))
    } else {
        None
    };
    let remote_field = if matches!(&command, crate::Cmds::Init { .. }) {
        Some(add_field(
            &mut layout,
            state,
            |state| &mut state.remote,
            "Remote URL (required)",
            font,
        ))
    } else {
        None
    };
    let name_field = if matches!(&command, crate::Cmds::Init { .. }) {
        Some(add_field(
            &mut layout,
            state,
            |state| &mut state.name,
            "Playlist name (optional)",
            font,
        ))
    } else {
        None
    };
    let message_field = if matches!(&command, crate::Cmds::Add { .. } | crate::Cmds::Push { .. }) {
        Some(add_field(
            &mut layout,
            state,
            |state| &mut state.message,
            "Commit message (optional)",
            font,
        ))
    } else {
        None
    };
    let default_player_field = if matches!(&command, crate::Cmds::Cfg { .. }) {
        Some(add_field_value(
            &mut layout,
            state,
            |state| &mut state.default_player,
            "Default player",
            font,
            if let Some(p) = config.default_player {
                p
            }
            else { "".to_string() }
        ))
    } else {
        None
    };
    let (hours_field, minutes_field) = if matches!(&command, crate::Cmds::Cfg { .. }) {
        let f = add_time_field_value(
            &mut layout,
            "Update cooldown",
            font,
            ((config.update_cooldown_m.unwrap_or(0) / 60) as i32, (config.update_cooldown_m.unwrap_or(0) % 60) as i32)
        );
        (Some(f.0), Some(f.1))
    } else {
        (None, None)
    };
    if matches!(
        &command,
        crate::Cmds::Add { .. } | crate::Cmds::Push { .. } | crate::Cmds::Init { .. }
    ) {
        let state_for_force = Arc::clone(state);
        let checkbox = CheckBox::new("Force push")
            .on_toggled(move |checked| state_for_force.write().unwrap().force_push = checked)
            .build();
        checkbox.set_font(font);
        layout.add_widget(Box::new(checkbox));
    }
    if matches!(&command, crate::Cmds::Init { .. }) {
        let state_for_reinit = Arc::clone(state);
        let checkbox = CheckBox::new("Reinitialize repository")
            .on_toggled(move |checked| state_for_reinit.write().unwrap().reinit = checked)
            .build();
        checkbox.set_font(font);
        layout.add_widget(Box::new(checkbox));
    }
    if matches!(&command, crate::Cmds::Reset { .. }) {
        let state_for_reset = Arc::clone(state);
        let checkbox = CheckBox::new("Reset default player")
            .on_toggled(move |checked| {
                state_for_reset.write().unwrap().reset_default_player = checked
            })
            .build();
        checkbox.set_font(font);
        layout.add_widget(Box::new(checkbox));
        let state_for_reset = Arc::clone(state);
        let checkbox = CheckBox::new("Reset update cooldown")
            .on_toggled(move |checked| {
                state_for_reset.write().unwrap().reset_update_cooldown = checked
            })
            .build();
        checkbox.set_font(font);
        layout.add_widget(Box::new(checkbox));
        let state_for_reset = Arc::clone(state);
        let checkbox = CheckBox::new("Reset update on play")
            .on_toggled(move |checked| {
                state_for_reset
                    .write()
                    .unwrap()
                    .reset_disable_update_on_play = checked
            })
            .build();
        checkbox.set_font(font);
        layout.add_widget(Box::new(checkbox));
    }
    if matches!(&command, crate::Cmds::Cfg { .. }) {
        let state_for_disable = Arc::clone(state);
        let checkbox = CheckBox::new("Disable update on play")
            .on_toggled(move |checked| {
                state_for_disable.write().unwrap().disable_update_on_play = checked
            })
            .build();
        checkbox.set_font(font);
        checkbox.set_checked(
            if let Some(d) = config.disable_update_on_play {
                if d { true }
                else { false }
            }
            else {
                false
            }
        );
        layout.add_widget(Box::new(checkbox));
    }
    let state_for_action = Arc::clone(state);
    let url_for_action = url_field.clone();
    let remote_for_action = remote_field.clone();
    let name_for_action = name_field.clone();
    let message_for_action = message_field.clone();
    let default_player_for_action = default_player_field.clone();
    let minutes_for_action = minutes_field.clone();
    let hours_for_action = hours_field.clone();
    let action = Rc::new(RefCell::new(
        PushButton::new(format!("{}", title))
            .on_clicked(move || {
                let (no_update, force_update) = update_values(&state_for_action);
                let command = match command {
                    crate::Cmds::Add { .. } => {
                        let url = sync_field(
                            url_for_action.as_ref().unwrap(),
                            &state_for_action,
                            |state| &mut state.url,
                        );
                        let message = sync_field(
                            message_for_action.as_ref().unwrap(),
                            &state_for_action,
                            |state| &mut state.message,
                        );
                        crate::Cmds::Add {
                            no_update,
                            force_update,
                            force_push: state_for_action.read().unwrap().force_push,
                            msg: (!message.is_empty()).then_some(message),
                            url,
                        }
                    }
                    crate::Cmds::Download { .. } => crate::Cmds::Download {
                        url: sync_field(
                            url_for_action.as_ref().unwrap(),
                            &state_for_action,
                            |state| &mut state.url,
                        ),
                    },
                    crate::Cmds::Update { .. } => crate::Cmds::Update {
                        force: force_update,
                    },
                    crate::Cmds::Push { .. } => crate::Cmds::Push {
                        no_update,
                        force_update,
                        force_push: state_for_action.read().unwrap().force_push,
                        msg: {
                            let message = sync_field(
                                message_for_action.as_ref().unwrap(),
                                &state_for_action,
                                |state| &mut state.message,
                            );
                            (!message.is_empty()).then_some(message)
                        },
                    },
                    crate::Cmds::Init { .. } => crate::Cmds::Init {
                        name: {
                            let name = sync_field(
                                name_for_action.as_ref().unwrap(),
                                &state_for_action,
                                |state| &mut state.name,
                            );
                            (!name.is_empty()).then_some(name)
                        },
                        force_push: state_for_action.read().unwrap().force_push,
                        reinit: state_for_action.read().unwrap().reinit,
                        remote: sync_field(
                            remote_for_action.as_ref().unwrap(),
                            &state_for_action,
                            |state| &mut state.remote,
                        ),
                    },
                    crate::Cmds::Cfg { .. } => crate::Cmds::Cfg {
                        default_player: {
                            let player = sync_field(
                                default_player_for_action.as_ref().unwrap(),
                                &state_for_action,
                                |state| &mut state.default_player,
                            );
                            (!player.is_empty()).then_some(player)
                        },
                        update_cooldown_m: Some(sync_field_number(
                            minutes_for_action.as_ref().unwrap(),
                            &state_for_action,
                            |state| &mut state.update_cooldown_m,
                        ) as u8),
                        update_cooldown_h: Some(sync_field_number(
                            hours_for_action.as_ref().unwrap(),
                            &state_for_action,
                            |state| &mut state.update_cooldown_h,
                        ) as u8),
                        disable_update_on_play: Some(
                            state_for_action.read().unwrap().disable_update_on_play,
                        ),
                    },
                    crate::Cmds::Reset { .. } => {
                        let state = state_for_action.read().unwrap();
                        crate::Cmds::Reset {
                            default_player: state.reset_default_player,
                            update_cooldown: state.reset_update_cooldown,
                            disable_update_on_play: state.reset_disable_update_on_play,
                        }
                    }
                    _ => return,
                };
                let b = if let crate::Cmds::Cfg { .. } = command { true } else if let crate::Cmds::Reset { .. } = command { true } else { false };
                run_command(command, lo, b);
            })
            .build(),
    ));
    action.borrow().set_font(font);
    if let Some(required_field) = url_field.as_ref().or(remote_field.as_ref()) {
        unsafe {
            qtrs::ffi::QWidget_setEnabled(action.borrow().widget_ptr(), false);
        }
        let action_for_field = Rc::downgrade(&action);
        let field_for_signal = Rc::clone(required_field);
        required_field.borrow_mut().connect_return_pressed(move || {
            field_for_signal.borrow_mut().refresh_text();
            if let Some(action) = action_for_field.upgrade() {
                unsafe {
                    qtrs::ffi::QWidget_setEnabled(
                        action.borrow().widget_ptr(),
                        !field_for_signal.borrow().text().trim().is_empty(),
                    );
                }
            }
        });
        let action_for_timer = Rc::downgrade(&action);
        let field_for_timer = Rc::clone(required_field);
        let timer = Timer::new(50)
            .on_timeout(move || {
                field_for_timer.borrow_mut().refresh_text();
                if let Some(action) = action_for_timer.upgrade() {
                    unsafe {
                        qtrs::ffi::QWidget_setEnabled(
                            action.borrow().widget_ptr(),
                            !field_for_timer.borrow().text().trim().is_empty(),
                        );
                    }
                }
            })
            .build();
        std::mem::forget(timer);
    }
    layout.add_widget(Box::new(SharedButton(action)));
    layout.add_spacer(Spacer::vertical_expanding());
    widget.set_layout(&layout);
    std::mem::forget(layout);
    Box::new(widget)
}

pub fn other_action(config: &Config, _state: &Arc<RwLock<State>>, font: &Font, lo: LogOptions) -> Box<dyn AsWidget> {
    let (mut widget, mut layout) = page();

    let def_p = &config.default_player;
    let def_p_label = Label::new(
        format!(
            "Default player: {}",
            if let Some(p) = def_p {
                format!("\"{}\"", p)
            } else {
                "none".to_string()
            }
        )
    )
        .build();
    def_p_label.set_font(font);
    layout.add_widget(Box::new(def_p_label));

    let l_u = &config.last_updated;
    let l_u_label = Label::new(
        format!(
            "Last updated: {}",
            if let Some(p) = l_u {
                format!("\"{}\"", p)
            } else {
                "never".to_string()
            }
        )
    )
        .build();
    l_u_label.set_font(font);
    layout.add_widget(Box::new(l_u_label));

    let cd = &config.update_cooldown_m;
    let cd_label = Label::new(
        format!(
            "Update cooldown: {}",
            if let Some(p) = cd {
                format!("\"{} hours and {} minutes\"", p / 60, p % 60)
            } else {
                "no cooldown".to_string()
            }
        )
    )
        .build();
    cd_label.set_font(font);
    layout.add_widget(Box::new(cd_label));

    let du = &config.disable_update_on_play;
    let du_label = Label::new(
        if let Some(p) = du && !p {
            "Update on play is enabled"
        } else {
            "Update on play is disabled"
        }
    )
        .build();
    du_label.set_font(font);
    layout.add_widget(Box::new(du_label));

    let reset_all = Rc::new(RefCell::new(
        PushButton::new("Reset all configuration")
            .on_clicked(move || {
                run_command(crate::Cmds::Reset {
                    default_player: true,
                    update_cooldown: true,
                    disable_update_on_play: true,
                }, lo, true);
            })
            .build(),
    ));
    reset_all.borrow().set_font(font);
    layout.add_widget(Box::new(SharedButton(reset_all)));

    layout.add_spacer(Spacer::vertical_expanding());
    layout.add_widget(Box::new( {
        let l = Label::new(
            format!("<div style='text-align:center'>{}</div>", 
            crate::Cli::command().get_long_about().unwrap_or_default().to_string())
        ).build();
        l.set_font(&Font::new().point_size(12).build());
        l
    }));
    layout.add_widget(Box::new( {
        let l = Label::new(
            format!("<div style='text-align:center'>{}</div>", 
            crate::Cli::command().get_name().to_string() + " " +
            &crate::Cli::command().get_version().unwrap_or_default().to_string())
        ).build();
        l.set_font(&Font::new().point_size(12).build());
        l
    }));
    widget.set_layout(&layout);
    std::mem::forget(layout);
    Box::new(widget)
}

fn shortcut_page(state: &Arc<RwLock<State>>, font: &Font, lo: LogOptions) -> Box<dyn AsWidget> {
    let (mut widget, mut layout) = page();
    let name_field = add_field(
        &mut layout,
        state,
        |state| &mut state.shortcut_name,
        "Shortcut name",
        font,
    );

    let state_for_folder = Arc::clone(state);
    let folder_button = Rc::new(RefCell::new(
        PushButton::new("Choose output folder (current directory)").build(),
    ));
    let button_for_folder = Rc::downgrade(&folder_button);
    folder_button.borrow_mut().connect_clicked(move || {
        if let Some(folder) = FileDialog::select_directory(None, "Choose shortcut output folder", "") {
            state_for_folder.write().unwrap().output_dir.clone_from(&folder);
            if let Some(button) = button_for_folder.upgrade() {
                button
                    .borrow_mut()
                    .set_text(format!("Output folder: {folder}"));
            }
        }
    });
    folder_button.borrow().set_font(font);
    layout.add_widget(Box::new(SharedButton(Rc::clone(&folder_button))));

    let state_for_action = Arc::clone(state);
    let name_for_action = Rc::clone(&name_field);
    let action = Rc::new(RefCell::new(
        PushButton::new("Create shortcut")
            .on_clicked(move || {
                let name = sync_field(
                    &name_for_action,
                    &state_for_action,
                    |state| &mut state.shortcut_name,
                );
                let output_dir = state_for_action.read().unwrap().output_dir.clone();
                run_command(crate::Cmds::Shortcut {
                    name,
                    output_dir: (!output_dir.is_empty()).then_some(output_dir.into()),
                }, lo, false);
            })
            .build(),
    ));
    action.borrow().set_font(font);
    unsafe {
        qtrs::ffi::QWidget_setEnabled(action.borrow().widget_ptr(), false);
    }
    let action_for_timer = Rc::downgrade(&action);
    let name_for_timer = Rc::clone(&name_field);
    let timer = Timer::new(50)
        .on_timeout(move || {
            name_for_timer.borrow_mut().refresh_text();
            let valid = crate::shortcut_name_is_valid(name_for_timer.borrow().text());
            if let Some(action) = action_for_timer.upgrade() {
                unsafe {
                    qtrs::ffi::QWidget_setEnabled(action.borrow().widget_ptr(), valid);
                }
            }
        })
        .build();
    std::mem::forget(timer);
    layout.add_widget(Box::new(SharedButton(action)));

    layout.add_spacer(Spacer::vertical_expanding());
    widget.set_layout(&layout);
    std::mem::forget(layout);
    Box::new(widget)
}

struct SharedStack(Rc<RefCell<StackedWidget>>);

impl AsWidget for SharedStack {
    fn widget_ptr(&self) -> *mut qtrs::ffi::QWidget {
        self.0.borrow().widget_ptr()
    }

    fn set_has_parent(&mut self) {
        self.0.borrow_mut().set_has_parent();
    }
}

pub fn run(c: &Config, lo: LogOptions) {
    let state = Arc::new(RwLock::new(State::default()));
    let font = Font::new().point_size(20).build();
    let output = Arc::new(Mutex::new(String::new()));
    let output_for_sink = Arc::clone(&output);
    set_log_sink(Some(Arc::new(move |chunk| {
        append_output(&mut output_for_sink.lock().unwrap(), chunk);
    })));

    let app = Application::new();
    app.set_icon("assets/plg.png");

    let window = MainWindow::new().window_title("Playlist with Git").build();

    let stack = Rc::new(RefCell::new(StackedWidget::new().build()));
    let page_selector = SharedComboBox(Rc::new(RefCell::new(ComboBox::new().build())));
    let pages = [
        ("Play", run_ui(c, &state, &font, lo, Rc::clone(&page_selector.0))),
        (
            "Download and push",
            other(
                &state,
                "Download and push",
                crate::Cmds::Add {
                    no_update: false,
                    force_update: false,
                    force_push: false,
                    msg: None,
                    url: String::new(),
                },
                &font,
                lo,
            ),
        ),
        (
            "Download",
            other(
                &state,
                "Download",
                crate::Cmds::Download { url: String::new() },
                &font,
                lo,
            ),
        ),
        (
            "Update",
            other(
                &state,
                "Update",
                crate::Cmds::Update { force: false },
                &font,
                lo,
            ),
        ),
        (
            "Push",
            other(
                &state,
                "Push",
                crate::Cmds::Push {
                    no_update: false,
                    force_update: false,
                    force_push: false,
                    msg: None,
                },
                &font,
                lo,
            ),
        ),
        (
            "Initialize",
            other(
                &state,
                "Initialize",
                crate::Cmds::Init {
                    name: None,
                    force_push: false,
                    reinit: false,
                    remote: String::new(),
                },
                &font,
                lo,
            ),
        ),
        (
            "Configure",
            other(
                &state,
                "Configure",
                crate::Cmds::Cfg {
                    default_player: None,
                    update_cooldown_m: None,
                    update_cooldown_h: None,
                    disable_update_on_play: None,
                },
                &font,
                lo,
            ),
        ),
        (
            "Reset",
            other(
                &state,
                "Reset selected",
                crate::Cmds::Reset {
                    default_player: false,
                    update_cooldown: false,
                    disable_update_on_play: false,
                },
                &font,
                lo,
            ),
        ),
        ("Shortcut", shortcut_page(&state, &font, lo)),
        (
            "Other...",
            other_action(c, &state, &font, lo),
        ),
    ];
    let page_names: Vec<&str> = pages.iter().map(|(name, _)| *name).collect();
    for (_, page) in pages {
        stack.borrow_mut().add_widget(page);
    }

    let stack_for_selection = Rc::clone(&stack);
    for p in page_names {
        page_selector.0.borrow_mut().add_item(p);
    }
    page_selector.0.borrow_mut().connect_current_index_changed(move |index| {
        stack_for_selection.borrow().set_current_index(index);
    });
    page_selector.set_font(&font);

    let output_view = Rc::new(RefCell::new(TextBrowser::new().build()));
    output_view.borrow().set_font(&Font::new().point_size(12).build());
    output_view.borrow().set_open_links(false);
    let output_for_timer = Arc::clone(&output);
    let output_view_for_timer = Rc::downgrade(&output_view);
    let output_timer = Timer::new(50)
        .on_timeout(move || {
            let text = output_for_timer.lock().unwrap().clone();
            if let Some(output_view) = output_view_for_timer.upgrade() {
                if output_view.borrow().plain_text() != text {
                    output_view.borrow().set_plain_text(&text);
                }
            }
        })
        .build();
    std::mem::forget(output_timer);

    let mut central = Widget::new().build();
    let mut layout = VBoxLayout::new();
    layout.add(page_selector);
    layout.add(SharedStack(stack));
    layout.add(SharedTextBrowser(Rc::clone(&output_view)));
    central.set_layout(&layout);
    std::mem::forget(layout);

    window.set_central_widget(&central);
    window.show();

    app.exec();
}
