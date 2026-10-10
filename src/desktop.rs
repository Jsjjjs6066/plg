use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    thread,
};

use crate::{Config, LocalConfig, load_config, load_local_config};
use chrono::TimeDelta;
use plg::{LogOptions, cwd, log, repo, set_log_sink};
use qtrs::{scrollarea::{SCROLL_BAR_ALWAYS_OFF, SCROLL_BAR_AS_NEEDED}, *};
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
    restart_requested: bool,
}

type UiTimers = Rc<RefCell<Vec<Timer>>>;

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
            restart_requested: false,
        }
    }
}

fn restart(state: Arc<RwLock<State>>) {
    state.write().unwrap().restart_requested = true;
}

fn update_options(state: Arc<RwLock<State>>, font: &Font, timers: &UiTimers) -> Rc<RefCell<ComboBox>> {
    let options = Rc::new(RefCell::new(ComboBox::new()
        .items(&["Default update", "No update", "Force update"])
        .build()));
    let state_for_signal = Arc::clone(&state);
    options.borrow_mut().connect_current_index_changed(move |index| {
        state_for_signal.write().unwrap().update_option = index as u8;
    });
    options.borrow().set_font(font);
    let options_for_timer = Rc::downgrade(&options);
    let state_for_timer = Arc::clone(&state);
    let timer = Timer::new(50)
        .on_timeout(move || {
            if let Some(options) = options_for_timer.upgrade() {
                let index = state_for_timer.read().unwrap().update_option as i32;
                options.borrow().set_current_index(index);
            }
        })
        .build();
    timers.borrow_mut().push(timer);
    options
}

fn force_update_checkbox(state: Arc<RwLock<State>>, font: &Font, timers: &UiTimers) -> Rc<RefCell<CheckBox>> {
    let state_for_checkbox = Arc::clone(&state);
    let checkbox = Rc::new(RefCell::new(CheckBox::new("Force update")
        .on_toggled(move |checked| {
            state_for_checkbox.write().unwrap().update_option = if checked { 2 } else { 0 };
        })
        .build()));
    checkbox.borrow().set_font(font);
    checkbox.borrow_mut().set_checked(state.read().unwrap().update_option == 2);
    let checkbox_for_timer = Rc::downgrade(&checkbox);
    let state_for_timer = Arc::clone(&state);
    let timer = Timer::new(50)
        .on_timeout(move || {
            if let Some(checkbox) = checkbox_for_timer.upgrade() {
                let checked = state_for_timer.read().unwrap().update_option == 2;
                if checkbox.borrow().is_checked() != checked {
                    checkbox.borrow_mut().set_checked(checked);
                }
            }
        })
        .build();
    timers.borrow_mut().push(timer);
    checkbox
}

fn update_values(state: Arc<RwLock<State>>) -> (bool, bool) {
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

fn set_window_icon(window: &MainWindow, icon_path: &str) {
    cxx::let_cxx_string!(path = icon_path);
    unsafe {
        qtrs::ffi::QWidget_setWindowIcon(window.widget_ptr(), &path);
    }
}

fn app_icon_path() -> Result<PathBuf, std::io::Error> {
    #[cfg(windows)]
    let icon_name = "plg.ico";
    #[cfg(not(windows))]
    let icon_name = "plg.png";

    let executable = std::env::current_exe()?;
    let executable_dir = executable.parent().ok_or_else(|| {
        std::io::Error::other("Unable to determine the application directory.")
    })?;

    for directory in executable_dir.ancestors() {
        for candidate in [
            directory.join("assets").join(icon_name),
            directory.join(icon_name),
        ] {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("Unable to find {icon_name} alongside the application executable."),
    ))
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

fn run_command(state: Arc<RwLock<State>>, command: crate::Cmds, lo: LogOptions, restart_after: bool) {
    thread::spawn(move || {
        if let Err(error) = crate::run(command, lo) {
            log!(lo, "Error: {error}");
        } else if restart_after {
            restart(state);
        }
    });
}

fn run_shortcut_command(command: crate::Cmds, lo: LogOptions, timers: &UiTimers) {
    let finished = Arc::new(AtomicBool::new(false));
    let finished_for_worker = Arc::clone(&finished);
    thread::spawn(move || {
        if let Err(error) = crate::run(command, lo) {
            log!(lo, "Error: {error}");
        }
        finished_for_worker.store(true, Ordering::Release);
    });

    let dialog = Rc::new(RefCell::new(Some(
        ProgressDialog::new("Creating shortcut...", "", 0, 0).build(),
    )));
    if let Some(dialog) = dialog.borrow().as_ref() {
        dialog.set_minimum_duration(0);
        dialog.show();
    }

    let dialog_for_timer = Rc::clone(&dialog);
    let timer = Timer::new(50)
        .on_timeout(move || {
            if finished.load(Ordering::Acquire) {
                if let Some(dialog) = dialog_for_timer.borrow_mut().take() {
                    dialog.close();
                }
            }
        })
        .build();
    timers.borrow_mut().push(timer);
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

struct SharedCheckBox(Rc<RefCell<CheckBox>>);

impl AsWidget for SharedCheckBox {
    fn widget_ptr(&self) -> *mut qtrs::ffi::QWidget {
        self.0.borrow().widget_ptr()
    }

    fn set_has_parent(&mut self) {
        self.0.borrow_mut().set_has_parent();
    }
}

fn add_field(
    layout: &mut VBoxLayout,
    state: Arc<RwLock<State>>,
    field: fn(&mut State) -> &mut String,
    label: &str,
    font: &Font,
) -> Rc<RefCell<LineEdit>> {
    let label_widget = Label::new(label).build();
    label_widget.set_font(font);
    layout.add_widget(Box::new(label_widget));

    let initial_value = field(&mut state.write().unwrap()).clone();
    let edit = Rc::new(RefCell::new(LineEdit::new(initial_value).build()));
    edit.borrow().set_font(font);
    let state_for_edit = Arc::clone(&state);
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
    state: Arc<RwLock<State>>,
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
    let state_for_edit = Arc::clone(&state);
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
    std::mem::forget(vb);
    
    layout.add_widget(Box::new(wid));
    (hours, minutes)
}

fn sync_field(
    edit: &Rc<RefCell<LineEdit>>,
    state: Arc<RwLock<State>>,
    field: fn(&mut State) -> &mut String,
) -> String {
    edit.borrow_mut().refresh_text();
    let value = edit.borrow().text().to_owned();
    field(&mut state.write().unwrap()).clone_from(&value);
    value
}

fn sync_field_number(
    edit: &Rc<RefCell<SpinBox>>,
    state: Arc<RwLock<State>>,
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

fn add_update_controls(
    layout: &mut VBoxLayout,
    state: Arc<RwLock<State>>,
    font: &Font,
    force_checkbox: bool,
    timers: &UiTimers,
) {
    let label = Label::new("Update options").build();
    label.set_font(font);
    layout.add_widget(Box::new(label));
    if force_checkbox {
        layout.add_widget(Box::new(SharedCheckBox(force_update_checkbox(state.clone(), font, timers))));
    } else {
        layout.add_widget(Box::new(SharedComboBox(update_options(state.clone(), font, timers))));
    }
}

pub fn run_ui(c: &Config, local: &LocalConfig, state: Arc<RwLock<State>>, font: &Font, lo: LogOptions, selector: Rc<RefCell<ComboBox>>, timers: &UiTimers) -> Box<dyn AsWidget> {
    let (mut widget, mut layout) = page();
    add_update_controls(&mut layout, state.clone(), font, false, timers);

    let mut run_in_w = Widget::new().build();
    let mut run_in_l = GridLayout::new();

    let now = chrono::Utc::now().naive_utc();
    let cooldown = local.update_cooldown_m.or(c.update_cooldown_m).unwrap_or_default();
    if (now - TimeDelta::minutes(cooldown as i64)) < local.last_updated.unwrap_or_default() {
                let l = Label::new(format!(
                    "By default skipping update because of the update cooldown of {} minutes. Use the update options to override this behavior. Next update will be available in {} minutes.",
                cooldown,
                (local.last_updated.unwrap_or_default() + TimeDelta::minutes(cooldown as i64) - now).num_minutes()
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

    let state_for_default_player = Arc::clone(&state);
    let default_player = if let Some(player) = local.default_player.clone().or(c.default_player.clone()) {
        PushButton::new(format!("Play in {player}"))
            .on_clicked(move || {
                run_command(state_for_default_player.clone(), crate::Cmds::Play {
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

    let state_for_edit = state.clone();
    let player_field = {
        let edit = Rc::new(RefCell::new(LineEdit::new("").build()));
        edit.borrow().set_font(font);
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
        state.clone(),
        |state| &mut state.args,
        "Player arguments (optional)",
        font,
    );
    {
        let state_for_play = Arc::clone(&state);
        let player_for_play = Rc::clone(&player_field);
        let args_for_play = Rc::clone(&args_field);
        let play_in = PushButton::new("Play in...")
            .on_clicked(move || {
                let (no_update, force_update) = update_values(state_for_play.clone());
                let player = sync_field(&player_for_play, state_for_play.clone(), |state| &mut state.player);
                let args = sync_field(&args_for_play, state_for_play.clone(), |state| &mut state.args);
                run_command(state_for_play.clone(), crate::Cmds::Play {
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
    state: Arc<RwLock<State>>,
    title: &str,
    command: crate::Cmds,
    font: &Font,
    lo: LogOptions,
    local: bool,
    timers: &UiTimers,
) -> Box<dyn AsWidget> {
    let config = load_config().unwrap_or_default();
    let local_config = load_local_config().unwrap_or_default();
    let default_player = if local { local_config.default_player.clone() } else { config.default_player.clone() };
    let update_cooldown_m = if local { local_config.update_cooldown_m } else { config.update_cooldown_m };
    let disable_update_on_play = if local { local_config.disable_update_on_play } else { config.disable_update_on_play };
    let (mut widget, mut layout) = page();
    if matches!(
        &command,
        crate::Cmds::Add { .. } | crate::Cmds::Update { .. } | crate::Cmds::Push { .. }
    ) {
        add_update_controls(
            &mut layout,
            state.clone(),
            font,
            matches!(&command, crate::Cmds::Update { .. }),
            timers,
        );
    }
    let url_field = if matches!(
        &command,
        crate::Cmds::Add { .. } | crate::Cmds::Download { .. }
    ) {
        Some(add_field(
            &mut layout,
            state.clone(),
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
            state.clone(),
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
            state.clone(),
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
            state.clone(),
            |state| &mut state.message,
            "Commit message (optional)",
            font,
        ))
    } else {
        None
    };
    let default_player_field = if matches!(&command, crate::Cmds::Cfg { .. } | crate::Cmds::CfgLocal { .. }) {
        Some(add_field_value(
            &mut layout,
            state.clone(),
            |state| &mut state.default_player,
            "Default player",
            font,
            if let Some(p) = default_player {
                p
            }
            else { "".to_string() }
        ))
    } else {
        None
    };
    let (hours_field, minutes_field) = if matches!(&command, crate::Cmds::Cfg { .. } | crate::Cmds::CfgLocal { .. }) {
        let f = add_time_field_value(
            &mut layout,
            "Update cooldown",
            font,
            ((update_cooldown_m.unwrap_or(0) / 60) as i32, (update_cooldown_m.unwrap_or(0) % 60) as i32)
        );
        (Some(f.0), Some(f.1))
    } else {
        (None, None)
    };
    if matches!(
        &command,
        crate::Cmds::Add { .. } | crate::Cmds::Push { .. } | crate::Cmds::Init { .. }
    ) {
        let state_for_force = Arc::clone(&state);
        let checkbox = CheckBox::new("Force push")
            .on_toggled(move |checked| state_for_force.write().unwrap().force_push = checked)
            .build();
        checkbox.set_font(font);
        layout.add_widget(Box::new(checkbox));
    }
    if matches!(&command, crate::Cmds::Init { .. }) {
        let state_for_reinit = Arc::clone(&state);
        let checkbox = CheckBox::new("Reinitialize repository")
            .on_toggled(move |checked| state_for_reinit.write().unwrap().reinit = checked)
            .build();
        checkbox.set_font(font);
        layout.add_widget(Box::new(checkbox));
    }
    if matches!(&command, crate::Cmds::Reset { .. } | crate::Cmds::ResetLocal { .. }) {
        let state_for_reset = Arc::clone(&state);
        let checkbox = CheckBox::new("Reset default player")
            .on_toggled(move |checked| {
                state_for_reset.write().unwrap().reset_default_player = checked
            })
            .build();
        checkbox.set_font(font);
        layout.add_widget(Box::new(checkbox));
        let state_for_reset = Arc::clone(&state);
        let checkbox = CheckBox::new("Reset update cooldown")
            .on_toggled(move |checked| {
                state_for_reset.write().unwrap().reset_update_cooldown = checked
            })
            .build();
        checkbox.set_font(font);
        layout.add_widget(Box::new(checkbox));
        let state_for_reset = Arc::clone(&state);
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
    if matches!(&command, crate::Cmds::Cfg { .. } | crate::Cmds::CfgLocal { .. }) {
        let state_for_disable = Arc::clone(&state);
        let checkbox = CheckBox::new("Disable update on play")
            .on_toggled(move |checked| {
                state_for_disable.write().unwrap().disable_update_on_play = checked
            })
            .build();
        checkbox.set_font(font);
        checkbox.set_checked(
            if let Some(d) = disable_update_on_play {
                if d { true }
                else { false }
            }
            else {
                false
            }
        );
        layout.add_widget(Box::new(checkbox));
    }
    let state_for_action = Arc::clone(&state);
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
                let (no_update, force_update) = update_values(state_for_action.clone());
                let command = match command {
                    crate::Cmds::Add { .. } => {
                        let url = sync_field(
                            url_for_action.as_ref().unwrap(),
                            state_for_action.clone(),
                            |state| &mut state.url,
                        );
                        let message = sync_field(
                            message_for_action.as_ref().unwrap(),
                            state_for_action.clone(),
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
                            state_for_action.clone(),
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
                                state_for_action.clone(),
                                |state| &mut state.message,
                            );
                            (!message.is_empty()).then_some(message)
                        },
                    },
                    crate::Cmds::Init { .. } => crate::Cmds::Init {
                        name: {
                            let name = sync_field(
                                name_for_action.as_ref().unwrap(),
                                state_for_action.clone(),
                                |state| &mut state.name,
                            );
                            (!name.is_empty()).then_some(name)
                        },
                        force_push: state_for_action.read().unwrap().force_push,
                        reinit: state_for_action.read().unwrap().reinit,
                        remote: sync_field(
                            remote_for_action.as_ref().unwrap(),
                            state_for_action.clone(),
                            |state| &mut state.remote,
                        ),
                    },
                    crate::Cmds::Cfg { .. } | crate::Cmds::CfgLocal { .. } => {
                        let configured = crate::Cmds::Cfg {
                            default_player: {
                            let player = sync_field(
                                default_player_for_action.as_ref().unwrap(),
                                state_for_action.clone(),
                                |state| &mut state.default_player,
                            );
                            (!player.is_empty()).then_some(player)
                        },
                        update_cooldown_m: Some(sync_field_number(
                            minutes_for_action.as_ref().unwrap(),
                            state_for_action.clone(),
                            |state| &mut state.update_cooldown_m,
                        ) as u8),
                        update_cooldown_h: Some(sync_field_number(
                            hours_for_action.as_ref().unwrap(),
                            state_for_action.clone(),
                            |state| &mut state.update_cooldown_h,
                        ) as u8),
                            disable_update_on_play: Some(
                            state_for_action.read().unwrap().disable_update_on_play,
                            ),
                        };
                        if matches!(command, crate::Cmds::CfgLocal { .. }) {
                            if let crate::Cmds::Cfg { default_player, update_cooldown_m, update_cooldown_h, disable_update_on_play } = configured {
                                crate::Cmds::CfgLocal { default_player, update_cooldown_m, update_cooldown_h, disable_update_on_play }
                            } else { unreachable!() }
                        } else { configured }
                    },
                    crate::Cmds::Reset { .. } | crate::Cmds::ResetLocal { .. } => {
                        let state = state_for_action.read().unwrap();
                        if matches!(command, crate::Cmds::ResetLocal { .. }) {
                            crate::Cmds::ResetLocal {
                                default_player: state.reset_default_player,
                                update_cooldown: state.reset_update_cooldown,
                                disable_update_on_play: state.reset_disable_update_on_play,
                            }
                        } else { crate::Cmds::Reset {
                            default_player: state.reset_default_player,
                            update_cooldown: state.reset_update_cooldown,
                            disable_update_on_play: state.reset_disable_update_on_play,
                        }}
                    }
                    _ => return,
                };
                let b = matches!(command, crate::Cmds::Cfg { .. } | crate::Cmds::CfgLocal { .. } | crate::Cmds::Reset { .. } | crate::Cmds::ResetLocal { .. });
                run_command(state_for_action.clone(), command, lo, b);
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
        timers.borrow_mut().push(timer);
    }
    layout.add_widget(Box::new(SharedButton(action)));
    layout.add_spacer(Spacer::vertical_expanding());
    widget.set_layout(&layout);
    std::mem::forget(layout);
    Box::new(widget)
}

pub fn other_action(config: &Config, state: Arc<RwLock<State>>, font: &Font, lo: LogOptions) -> Box<dyn AsWidget> {
    let (mut widget, mut layout) = page();
    let local = load_local_config().unwrap_or_default();

    let global_heading = Label::new("Global configuration").build();
    global_heading.set_font(&Font::new().point_size(22).bold(true).build());
    layout.add_widget(Box::new(global_heading));

    let mut swidget = Widget::new().build();
    let mut slayout = VBoxLayout::new();

    let def_p = &config.default_player;
    let def_p_label = Label::new(
        format!(
            "Default player: {}",
            if let Some(p) = def_p {
                format!("\"{}\"", p)
            }
            else {
                "none".to_string()
            }
        )
    )
        .build();
    def_p_label.set_font(font);
    slayout.add_widget(Box::new(def_p_label));

    let cd = &config.update_cooldown_m;
    let cd_label = Label::new(
        format!(
            "Update cooldown: {}",
            if let Some(p) = cd {
                format!("\"{} hours and {} minutes\"", p / 60, p % 60)
            }
            else {
                "no cooldown".to_string()
            }
        )
    )
        .build();
    cd_label.set_font(font);
    slayout.add_widget(Box::new(cd_label));

    let du = &config.disable_update_on_play;
    let du_label = Label::new(
        if let Some(p) = du && *p {
            "Update on play is disabled"
        }
        else {
            "Update on play is enabled"
        }
    )
        .build();
    du_label.set_font(font);
    slayout.add_widget(Box::new(du_label));

    let state_for_reset_global = Arc::clone(&state);
    let reset_global = Rc::new(RefCell::new(
        PushButton::new("Reset all global configuration")
            .on_clicked(move || {
                run_command(state_for_reset_global.clone(), crate::Cmds::Reset {
                    default_player: true,
                    update_cooldown: true,
                    disable_update_on_play: true,
                }, lo, true);
            })
            .build(),
    ));
    reset_global.borrow().set_font(font);
    slayout.add_widget(Box::new(SharedButton(reset_global)));

    let local_heading = Label::new("Local configuration").build();
    local_heading.set_font(&Font::new().point_size(22).bold(true).build());
    slayout.add_widget(Box::new(local_heading));
    let local_player = Label::new(format!(
        "Default player: {}",
        local.default_player.as_deref().unwrap_or("inherits global")
    )).build();
    local_player.set_font(font);
    slayout.add_widget(Box::new(local_player));
    let local_updated = Label::new(format!(
        "Last updated: {}",
        local.last_updated.map(|value| value.to_string()).unwrap_or_else(|| "never".to_string())
    )).build();
    local_updated.set_font(font);
    slayout.add_widget(Box::new(local_updated));
    let local_cooldown = Label::new(format!(
        "Update cooldown: {}",
        local.update_cooldown_m
            .map(|value| format!("{} hours and {} minutes", value / 60, value % 60))
            .unwrap_or_else(|| "inherits global".to_string())
    )).build();
    local_cooldown.set_font(font);
    slayout.add_widget(Box::new(local_cooldown));
    let local_play = Label::new(if local.disable_update_on_play == Some(false) {
        "Update on play: enabled (local)"
    }
    else if local.disable_update_on_play == Some(true) {
        "Update on play: disabled (local)"
    }
    else {
        "Update on play: inherits global"
    }).build();
    local_play.set_font(font);
    slayout.add_widget(Box::new(local_play));
    let state_for_reset_local = Arc::clone(&state);
    let reset_local = Rc::new(RefCell::new(
        PushButton::new("Reset all local configuration")
            .on_clicked(move || {
                run_command(state_for_reset_local.clone(), crate::Cmds::ResetLocal {
                    default_player: true,
                    update_cooldown: true,
                    disable_update_on_play: true,
                }, lo, true);
            })
            .build(),
    ));
    reset_local.borrow().set_font(font);
    slayout.add_widget(Box::new(SharedButton(reset_local)));

    swidget.set_layout(&slayout);
    let scroll = ScrollArea::new()
        .set_widget_resizable(true)
        .build();
    scroll.set_widget(&swidget);
    scroll.set_horizontal_scroll_bar_policy(SCROLL_BAR_ALWAYS_OFF);
    scroll.set_vertical_scroll_bar_policy(SCROLL_BAR_AS_NEEDED);
    layout.add_widget(Box::new(scroll));

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
    std::mem::forget(swidget);
    std::mem::forget(slayout);
    std::mem::forget(layout);
    Box::new(widget)
}

fn shortcut_page(
    state: Arc<RwLock<State>>,
    font: &Font,
    lo: LogOptions,
    last_output_dir: Option<String>,
    timers: &UiTimers,
) -> Box<dyn AsWidget> {
    let (mut widget, mut layout) = page();
    if let Some(last_output_dir) = last_output_dir {
        state.write().unwrap().output_dir = last_output_dir;
    }
    let name_field = add_field(
        &mut layout,
        state.clone(),
        |state| &mut state.shortcut_name,
        "Shortcut name",
        font,
    );

    let state_for_folder = Arc::clone(&state);
    let folder_button = Rc::new(RefCell::new(
        PushButton::new(format!(
            "Output directory: {}",
            if state.read().unwrap().output_dir.is_empty() {
                state_for_folder.write().unwrap().output_dir.clone_from(&cwd().to_string_lossy().to_string());
                "current directory".to_string();
                state.read().unwrap().output_dir.clone()
            } else {
                state.read().unwrap().output_dir.clone()
            }
        )).build(),
    ));
    let button_for_folder = Rc::downgrade(&folder_button);
    folder_button.borrow_mut().connect_clicked(move || {
        if let Some(folder) = FileDialog::select_directory(None, "Choose shortcut Output directory", "") {
            state_for_folder.write().unwrap().output_dir.clone_from(&folder);
            if let Some(button) = button_for_folder.upgrade() {
                button
                    .borrow_mut()
                    .set_text(format!("Output directory: {folder}"));
            }
        }
    });
    folder_button.borrow().set_font(font);
    layout.add_widget(Box::new(SharedButton(Rc::clone(&folder_button))));

    let state_for_action = Arc::clone(&state);
    let name_for_action = Rc::clone(&name_field);
    let timers_for_action = Rc::clone(timers);
    let action = Rc::new(RefCell::new(
        PushButton::new("Create shortcut")
            .on_clicked(move || {
                let name = sync_field(
                    &name_for_action,
                    state_for_action.clone(),
                    |state| &mut state.shortcut_name,
                );
                let output_dir = state_for_action.read().unwrap().output_dir.clone();
                run_shortcut_command(crate::Cmds::Shortcut {
                    name,
                    output_dir: (!output_dir.is_empty()).then_some(output_dir.into()),
                }, lo, &timers_for_action);
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
    timers.borrow_mut().push(timer);
    layout.add_widget(Box::new(SharedButton(action)));

    layout.add_spacer(Spacer::vertical_expanding());
    widget.set_layout(&layout);
    std::mem::forget(layout);
    Box::new(widget)
}

fn directory_button(state: Arc<RwLock<State>>, c: &Config, font: &Font) -> Rc<RefCell<PushButton>> {
    let directory = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .to_string_lossy()
        .into_owned();
    let button = Rc::new(RefCell::new(
        PushButton::new(format!("Working folder: {directory}"))
            .on_clicked({
                let last_directory = c.last_directory.clone().unwrap_or(directory);
                let state: Arc<RwLock<State>> = Arc::clone(&state);
                move || {
                    if let Some(folder) = FileDialog::select_directory(
                        None,
                        "Choose playlist folder",
                        &last_directory,
                    ) {
                        let mut config = load_config().unwrap_or_default();
                        config.last_directory = Some(folder.clone());
                        if let Err(error) = crate::save_config(&config) {
                            error_box(format!("Unable to save selected folder: {error}"));
                            return;
                        }
                        if let Err(error) = std::env::set_current_dir(&folder) {
                            error_box(format!("Unable to open selected folder: {error}"));
                            return;
                        }
                        restart(Arc::clone(&state));
                    }
                }
            })
            .build(),
    ));
    button.borrow().set_font(font);
    button
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

fn main_content(
    state: Arc<RwLock<State>>,
    c: &Config,
    lo: LogOptions,
    output_view: Rc<RefCell<Rc<RefCell<TextBrowser>>>>,
    timers: &UiTimers,
) -> Widget {
    let font = Font::new().point_size(20).build();
    let output_view = Rc::clone(&output_view.borrow());
    let stack = Rc::new(RefCell::new(StackedWidget::new().build()));
    let page_selector = SharedComboBox(Rc::new(RefCell::new(ComboBox::new().build())));
    let pages = [
        ("Play", run_ui(c, &load_local_config().unwrap_or_default(), state.clone(), &font, lo, Rc::clone(&page_selector.0), timers)),
        (
            "Download and push",
            other(
                state.clone(),
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
                false,
                timers,
            ),
        ),
        (
            "Download",
            other(
                state.clone(),
                "Download",
                crate::Cmds::Download { url: String::new() },
                &font,
                lo,
                false,
                timers,
            ),
        ),
        (
            "Update",
            other(
                state.clone(),
                "Update",
                crate::Cmds::Update { force: false },
                &font,
                lo,
                false,
                timers,
            ),
        ),
        (
            "Push",
            other(
                state.clone(),
                "Push",
                crate::Cmds::Push {
                    no_update: false,
                    force_update: false,
                    force_push: false,
                    msg: None,
                },
                &font,
                lo,
                false,
                timers,
            ),
        ),
        (
            "Initialize",
            other(
                state.clone(),
                "Initialize",
                crate::Cmds::Init {
                    name: None,
                    force_push: false,
                    reinit: false,
                    remote: String::new(),
                },
                &font,
                lo,
                false,
                timers,
            ),
        ),
        (
            "Configure globally",
            other(
                state.clone(),
                "Configure",
                crate::Cmds::Cfg {
                    default_player: None,
                    update_cooldown_m: None,
                    update_cooldown_h: None,
                    disable_update_on_play: None,
                },
                &font,
                lo,
                false,
                timers,
            ),
        ),
        (
            "Configure locally",
            other(
                state.clone(),
                "Configure locally",
                crate::Cmds::CfgLocal {
                    default_player: None,
                    update_cooldown_m: None,
                    update_cooldown_h: None,
                    disable_update_on_play: None,
                },
                &font,
                lo,
                true,
                timers,
            ),
        ),
        (
            "Reset globally",
            other(
                state.clone(),
                "Reset selected",
                crate::Cmds::Reset {
                    default_player: false,
                    update_cooldown: false,
                    disable_update_on_play: false,
                },
                &font,
                lo,
                false,
                timers,
            ),
        ),
        (
            "Reset locally",
            other(
                state.clone(),
                "Reset selected locally",
                crate::Cmds::ResetLocal {
                    default_player: false,
                    update_cooldown: false,
                    disable_update_on_play: false,
                },
                &font,
                lo,
                true,
                timers,
            ),
        ),
        ("Shortcut", shortcut_page(state.clone(), &font, lo, c.last_output_dir.clone(), timers)),
        (
            "Other...",
            other_action(c, state.clone(), &font, lo),
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

    let mut central = Widget::new().build();
    let mut layout = VBoxLayout::new();
    layout.add_widget(Box::new(SharedButton(directory_button(
        state.clone(),
        c,
        &font,
    ))));
    if repo().is_err() {
        let warning = Label::new("Warning: the current directory is not a Git repository.").build();
        warning.set_font(&Font::new().point_size(14).bold(true).build());
        layout.add_widget(Box::new(warning));
    }
    layout.add(page_selector);
    layout.add(SharedStack(stack));
    layout.add(SharedTextBrowser(Rc::clone(&output_view)));
    central.set_layout(&layout);
    std::mem::forget(layout);
    central
}

pub fn run(app: Arc<RwLock<Application>>, state: Arc<RwLock<State>>, c: &Config, lo: LogOptions) -> i32 {
    let output = Arc::new(Mutex::new(String::new()));
    let output_for_sink = Arc::clone(&output);
    set_log_sink(Some(Arc::new(move |chunk| {
        append_output(&mut output_for_sink.lock().unwrap(), chunk);
    })));

    let icon_path = match app_icon_path() {
        Ok(icon_path) => {
            app.read()
                .unwrap()
                .set_icon(&icon_path.to_string_lossy());
            Some(icon_path)
        }
        Err(error) => {
            error_box(error);
            None
        },
    };

    let main_window = MainWindow::new()
        .window_title("Playlist with Git")
        .build();
    if let Some(icon_path) = icon_path {
        set_window_icon(&main_window, &icon_path.to_string_lossy());
    }
    let window = Rc::new(RefCell::new(main_window));
    let create_output_view = || {
        let output_view = Rc::new(RefCell::new(TextBrowser::new().build()));
        output_view.borrow().set_font(&Font::new().point_size(12).build());
        output_view.borrow().set_open_links(false);
        output_view
    };
    let output_view = Rc::new(RefCell::new(create_output_view()));

    let output_for_timer = Arc::clone(&output);
    let output_view_for_timer = Rc::clone(&output_view);
    let output_timer = Timer::new(50)
        .on_timeout(move || {
            let text = output_for_timer.lock().unwrap().clone();
            let output_view = Rc::clone(&output_view_for_timer.borrow());
            if output_view.borrow().plain_text() != text {
                output_view.borrow().set_plain_text(&text);
            }
        })
        .build();

    let current_timers: UiTimers = Rc::new(RefCell::new(Vec::new()));
    let mut central = main_content(
        Arc::clone(&state),
        c,
        lo,
        Rc::clone(&output_view),
        &current_timers,
    );
    central.set_has_parent();
    window.borrow().set_central_widget(&central);
    std::mem::forget(central);
    window.borrow().show();

    let window_for_refresh = Rc::clone(&window);
    let timers_for_refresh = Rc::clone(&current_timers);
    let output_view_for_refresh = Rc::clone(&output_view);
    let refresh_timer = Timer::new(50)
        .on_timeout(move || {
            let restart_requested = {
                let mut state = state.write().unwrap();
                std::mem::take(&mut state.restart_requested)
            };
            if !restart_requested {
                return;
            }

            let config = match load_config() {
                Ok(config) => config,
                Err(error) => {
                    error_box(format!("Unable to reload configuration: {error}"));
                    return;
                }
            };
            if let Some(directory) = config.last_directory.as_deref() {
                if std::path::Path::new(directory).is_dir() {
                    if let Err(error) = std::env::set_current_dir(directory) {
                        error_box(format!("Unable to open configured folder: {error}"));
                    }
                }
            }
            let previous_timers =
                std::mem::take(&mut *timers_for_refresh.borrow_mut());
            drop(previous_timers);

            let output_view = create_output_view();
            *output_view_for_refresh.borrow_mut() = output_view;
            let mut central = main_content(
                Arc::clone(&state),
                &config,
                lo,
                Rc::clone(&output_view_for_refresh),
                &timers_for_refresh,
            );
            central.set_has_parent();
            window_for_refresh.borrow().set_central_widget(&central);
            std::mem::forget(central);
        })
        .build();

    let exit_code = app.read().unwrap().exec();
    drop(refresh_timer);
    drop(current_timers);
    drop(output_timer);
    drop(output_view);
    drop(window);
    exit_code
}
