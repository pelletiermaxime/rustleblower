//! Native file/profile controls; all filesystem and JSON work is off the UI thread.
use super::*;
use crate::firewall::transfer;
use std::path::{Path, PathBuf};

pub(super) struct Files {
    path: Entity<InputState>,
    profiles: Vec<PathBuf>,
    offset: usize,
}
#[derive(Clone, Copy)]
enum Operation {
    Load,
    Merge,
    Remove,
    Export,
    ExportProfile,
    CopyProfile,
    List,
}
enum Outcome {
    Draft(Document, String),
    Written,
    Clipboard(String),
    Listed(Vec<PathBuf>),
}
impl Files {
    pub(super) fn new(window: &mut Window, cx: &mut Context<FirewallView>) -> Self {
        Self {
            path: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(crate::i18n::placeholder(
                        "JSON/.profile file or profile directory…",
                        window,
                        cx,
                    ))
                    .validate(|text, _| text.len() <= 4096 && !text.contains('\0'))
            }),
            profiles: Vec::new(),
            offset: 0,
        }
    }
}
impl FirewallView {
    fn file_prompt(
        &mut self,
        save: bool,
        directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.loaded.is_none() {
            return;
        }
        self.busy = true;
        if save {
            let prompt = cx.prompt_for_new_path(Path::new("."), Some("system-firewall.json"));
            cx.spawn_in(window, async move |this, cx| {
                let result = prompt.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.busy = false;
                    match result {
                        Ok(Ok(Some(path))) => this.set_file_path(&path, window, cx),
                        Ok(Ok(None)) => {}
                        other => {
                            this.error = Some(crate::i18n::tr_args(
                                "Cannot choose an export path: %{other}",
                                &[("other", &format!("{:?}", other))],
                            ))
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            let prompt = cx.prompt_for_paths(PathPromptOptions {
                files: !directory,
                directories: directory,
                multiple: false,
                prompt: Some("Choose a system-firewall file or profile directory".into()),
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = prompt.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.busy = false;
                    match result {
                        Ok(Ok(Some(paths))) => {
                            if let Some(path) = paths.first() {
                                this.set_file_path(path, window, cx);
                            }
                        }
                        Ok(Ok(None)) => {}
                        other => {
                            this.error = Some(crate::i18n::tr_args(
                                "Cannot choose a firewall path: %{other}",
                                &[("other", &format!("{:?}", other))],
                            ))
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }
    fn set_file_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        match path
            .to_str()
            .filter(|value| value.len() <= 4096 && !value.contains('\0'))
        {
            Some(value) => self.files.path.update(cx, |input, cx| {
                input.set_value(value.to_owned(), window, cx)
            }),
            None => self.error = Some(crate::i18n::tr("Choose a UTF-8 path of at most 4096 bytes")),
        }
    }
    fn file_operation(
        &mut self,
        operation: Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let writing_draft = matches!(
            operation,
            Operation::Load | Operation::Merge | Operation::Remove
        );
        if self.busy || (writing_draft && !self.writable()) {
            return;
        }
        let Some(_) = &self.loaded else {
            return;
        };
        let text = self.input.read(cx).value().to_string();
        let path = PathBuf::from(self.files.path.read(cx).value().as_str());
        let chain = self.chain;
        self.busy = true;
        self.error = None;
        self.message = None;
        let work = cx.background_executor().spawn(async move {
            if matches!(operation, Operation::List) {
                let path = if path.as_os_str().is_empty() {
                    PathBuf::from(transfer::DEFAULT_PROFILES)
                } else {
                    path
                };
                return transfer::profiles(&path).map(Outcome::Listed);
            }
            let mut document = if matches!(operation, Operation::Load) {
                Document::parse(&transfer::read_text(&path)?)?
            } else {
                Document::parse(&text)?
            };
            match operation {
                Operation::Load => {
                    let text = document.json()?;
                    Ok(Outcome::Draft(document, text))
                }
                Operation::Merge | Operation::Remove => {
                    let profile = transfer::parse_profile(&transfer::read_text(&path)?)?;
                    if matches!(operation, Operation::Merge) {
                        document.apply_profile(profile)?;
                    } else {
                        document.remove_profile(&profile)?;
                    }
                    {
                        let text = document.json()?;
                        Ok(Outcome::Draft(document, text))
                    }
                }
                Operation::Export | Operation::ExportProfile | Operation::CopyProfile => {
                    let text = if matches!(operation, Operation::Export) {
                        document.json()?
                    } else {
                        let (g, c) = chain.ok_or("Select a chain to export as a profile")?;
                        transfer::profile_json(
                            document.chain(g, c).ok_or("Select a current chain")?,
                        )?
                    };
                    if matches!(operation, Operation::CopyProfile) {
                        Ok(Outcome::Clipboard(text))
                    } else {
                        transfer::write_new(&path, &text)?;
                        Ok(Outcome::Written)
                    }
                }
                Operation::List => unreachable!(),
            }
        });
        cx.spawn_in(window,async move|this,cx|{
            let result=work.await;
            let _=this.update_in(cx,|this,window,cx|{
                this.busy=false;
                match result {
                    Ok(Outcome::Draft(document,text))=>{
                            if let Some(loaded)=&mut this.loaded{loaded.document=document;}
                            this.last_text=text.clone();this.input.update(cx,|input,cx|input.set_value(text,window,cx));
                            this.form=None;this.stage=None;this.delete=None;this.selected_rule=None;this.offset=0;
                            if this.chain.is_none_or(|(g,c)|this.loaded.as_ref().is_none_or(|l|l.document.chain(g,c).is_none())) {
                                this.chain=this.loaded.as_ref().and_then(|l|l.document.system_rules.iter().enumerate().find_map(|(g,group)|(!group.chains.is_empty()).then_some((g,0))));
                            }
                            this.message=Some(crate::i18n::tr("File/profile changes kept in the local draft. Review and Apply sends the complete configuration."));
                    },
                    Ok(Outcome::Written)=>this.message=Some(crate::i18n::tr("Export saved to a new file; daemon unchanged")),
                    Ok(Outcome::Clipboard(text))=>{cx.write_to_clipboard(ClipboardItem::new_string(text));this.message=Some(crate::i18n::tr("Selected chain copied as a complete profile"));},
                    Ok(Outcome::Listed(profiles))=>{this.message=Some(crate::i18n::tr_args("%{arg0} profile files; select one, then merge or remove its rule IDs", &[("arg0", &format!("{}", profiles.len()))]));this.files.profiles=profiles;this.files.offset=0;},
                    Err(error)=>this.error=Some(error),
                }
                this.focus.focus(window,cx);cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(super) fn render_files(&self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.busy || self.loaded.is_none();
        let read_only = disabled || !self.writable();
        let selected = self.chain.is_some();
        let mut view=div().flex_1().min_h_0().flex().flex_col().gap_2()
            .child(div().text_sm().child(crate::i18n::tr("Configuration imports replace the local draft. Profiles match table/family/type/hook, set its policy and append nonduplicate rules. Removing a profile removes matching rule IDs and preserves policy. Review every change before Apply.")))
            .child(Input::new(&self.files.path).id("fw-file-path").disabled(disabled))
            .child(div().flex().gap_2()
                .child(Button::new("fw-file-browse").label(crate::i18n::tr("Choose file…")).disabled(disabled).on_click(cx.listener(|this,_,window,cx|this.file_prompt(false,false,window,cx))))
                .child(Button::new("fw-file-save-path").label(crate::i18n::tr("Choose export path…")).disabled(disabled).on_click(cx.listener(|this,_,window,cx|this.file_prompt(true,false,window,cx))))
                .child(Button::new("fw-profile-directory").label(crate::i18n::tr("Choose profile folder…")).disabled(disabled).on_click(cx.listener(|this,_,window,cx|this.file_prompt(false,true,window,cx)))))
            .child(div().flex().gap_2()
                .child(Button::new("fw-file-load").label(crate::i18n::tr("Load configuration into draft")).disabled(read_only).on_click(cx.listener(|this,_,window,cx|this.file_operation(Operation::Load,window,cx))))
                .child(Button::new("fw-profile-merge").label(crate::i18n::tr("Merge profile into draft")).disabled(read_only).on_click(cx.listener(|this,_,window,cx|this.file_operation(Operation::Merge,window,cx))))
                .child(Button::new("fw-profile-remove").label(crate::i18n::tr("Remove profile rule IDs")).disabled(read_only).on_click(cx.listener(|this,_,window,cx|this.file_operation(Operation::Remove,window,cx)))))
            .child(div().flex().gap_2()
                .child(Button::new("fw-file-export").label(crate::i18n::tr("Export full draft")).disabled(disabled).on_click(cx.listener(|this,_,window,cx|this.file_operation(Operation::Export,window,cx))))
                .child(Button::new("fw-profile-export").label(crate::i18n::tr("Export selected chain")).disabled(disabled||!selected).on_click(cx.listener(|this,_,window,cx|this.file_operation(Operation::ExportProfile,window,cx))))
                .child(Button::new("fw-profile-copy").label(crate::i18n::tr("Copy selected profile")).disabled(disabled||!selected).on_click(cx.listener(|this,_,window,cx|this.file_operation(Operation::CopyProfile,window,cx)))))
            .child(Button::new("fw-profiles-list").label(crate::i18n::tr("List profiles in folder (empty path uses OpenSnitch default)")).disabled(disabled).on_click(cx.listener(|this,_,window,cx|this.file_operation(Operation::List,window,cx))));
        let page = self
            .files
            .profiles
            .iter()
            .enumerate()
            .skip(self.files.offset)
            .take(firewall::PAGE)
            .map(|(index, path)| {
                let label = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Profile")
                    .to_owned();
                let path = path.to_string_lossy().to_string();
                Button::new(("fw-profile-file", index))
                    .label(crate::history::bounded(&label, 256))
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.files
                            .path
                            .update(cx, |input, cx| input.set_value(path.clone(), window, cx));
                        cx.notify();
                    }))
            });
        view = view.child(
            div()
                .id("fw-profile-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .children(page),
        );
        view.child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("fw-profiles-previous")
                        .label(crate::i18n::tr("Previous profiles"))
                        .disabled(disabled || self.files.offset == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.files.offset = this.files.offset.saturating_sub(firewall::PAGE);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("fw-profiles-next")
                        .label(crate::i18n::tr("Next profiles"))
                        .disabled(
                            disabled
                                || self.files.offset + firewall::PAGE >= self.files.profiles.len(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.files.offset += firewall::PAGE;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("fw-files-review")
                        .label(crate::i18n::tr("Review complete changes…"))
                        .disabled(read_only)
                        .on_click(cx.listener(|this, _, window, cx| this.review(window, cx))),
                ),
        )
        .into_any_element()
    }
}
