//! A separate bounded UI-side subscription worker; it never owns firewall policy.
pub mod attachment;
pub mod bulk;
pub mod collections;
mod download;
mod format;
pub mod limits;
pub mod model;
pub mod probe;
mod store;
pub mod transfer;
use model::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
use store::{Store, now};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Default)]
pub struct State {
    pub revision: u64,
    pub automatic: bool,
    pub local_ready: bool,
    pub archived_metadata: usize,
    pub active: Option<u64>,
    pub error: Option<String>,
    pub downloaded: Arc<AtomicU64>,
}
#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<Message>,
    state: watch::Receiver<State>,
    cancel: CancellationToken,
    ready: Arc<AtomicBool>,
    pub root: PathBuf,
}
pub struct Worker {
    handle: Handle,
    thread: Option<thread::JoinHandle<()>>,
}
pub enum Request {
    TestUrl(probe::Input),
    ReviewImport {
        path: PathBuf,
        replace: bool,
        defaults: bool,
        catalogue: bool,
    },
    ApplyImport(Box<transfer::Review>),
    Export {
        path: PathBuf,
        scope: bulk::Scope,
        encoding: transfer::Encoding,
    },
    ReviewBulk {
        scope: bulk::Scope,
        patch: bulk::Patch,
    },
    ApplyBulk(bulk::Review),
    Page(Query),
    Collections(collections::Query),
    Collection {
        group: Option<String>,
        format: Format,
    },
    Get(u64),
    Save {
        previous: Option<(u64, u64)>,
        spec: Spec,
    },
    SaveChecked {
        previous: Option<(u64, u64)>,
        spec: Spec,
        settings_revision: u64,
    },
    Remove {
        id: u64,
        revision: u64,
    },
    Settings,
    SaveSettings {
        revision: u64,
        settings: Settings,
    },
    Refresh {
        id: u64,
        force: bool,
    },
    Cancel,
    Runtime(bool),
    Logs(u64),
    Preview(u64),
}
pub enum Reply {
    UrlTest(probe::Report),
    ImportReview(Box<transfer::Review>),
    Imported(usize),
    Exported(usize),
    BulkReview(bulk::Review),
    BulkApplied(usize),
    Page(Page),
    Collections(collections::Page),
    Collection(Box<collections::Collection>),
    Row(Box<Row>),
    Saved(u64),
    Settings(u64, Settings),
    Logs(Vec<String>),
    Preview(String),
    Done,
}
struct Message {
    request: Request,
    reply: oneshot::Sender<Result<Reply, String>>,
}
impl Handle {
    pub fn state(&self) -> State {
        self.state.borrow().clone()
    }
    pub fn local_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::Relaxed);
    }
    pub fn request(
        &self,
        request: Request,
    ) -> Result<oneshot::Receiver<Result<Reply, String>>, String> {
        match &request {
            Request::ReviewImport { path, .. } => transfer::validate_path(path)?,
            Request::Export { path, scope, .. } => transfer::validate_export(scope, path)?,
            Request::ApplyImport(review) => review.validate()?,
            Request::ReviewBulk { scope, patch } => {
                scope.validate()?;
                patch.validate()?;
            }
            Request::ApplyBulk(review) => review.validate()?,
            Request::Save { spec, .. } | Request::SaveChecked { spec, .. } => spec.validate()?,
            Request::SaveSettings { settings, .. } => settings.validate()?,
            Request::Collections(q)
                if q.search.len() > 256 || q.offset > collections::MAX_LINKS + 5 =>
            {
                return Err("Invalid collection query".into());
            }
            Request::Collection {
                group: Some(group), ..
            } if group.len() > 64 => {
                return Err("Invalid collection target".into());
            }
            Request::Page(q) if q.search.len() > 256 || q.offset > MAX_RECORDS => {
                return Err("Invalid subscription query".into());
            }
            _ => {}
        }
        let (tx, rx) = oneshot::channel();
        self.tx.try_send(Message { request, reply: tx }).map_err(
            |_| "Subscription worker is busy or unavailable; retry after its current operation",
        )?;
        Ok(rx)
    }
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}
impl Worker {
    pub fn start(root: &Path) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let (state_tx, state) = watch::channel(State::default());
        let cancel = CancellationToken::new();
        let ready = Arc::new(AtomicBool::new(false));
        let handle = Handle {
            tx,
            state,
            cancel: cancel.clone(),
            ready: ready.clone(),
            root: root.to_owned(),
        };
        let path = root.to_owned();
        let errors = state_tx.clone();
        let fallback = state_tx.clone();
        let thread = thread::Builder::new()
            .name("rustleblower-subscriptions".into())
            .spawn(move || {
                let result = (|| -> Result<(), String> {
                    let store = Store::open(&path)?;
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .max_blocking_threads(2)
                        .enable_all()
                        .build()
                        .map_err(|e| e.to_string())?;
                    let result = rt.block_on(run(store, rx, state_tx, cancel, ready));
                    rt.shutdown_timeout(Duration::from_millis(250));
                    result
                })();
                if let Err(error) = result {
                    errors.send_modify(|s| {
                        s.error = Some(crate::history::bounded(&error, 1024));
                        s.revision += 1;
                        s.active = None;
                    });
                }
            });
        let thread = match thread {
            Ok(t) => Some(t),
            Err(error) => {
                fallback.send_modify(|s| {
                    s.error = Some(format!("Cannot start subscription worker: {error}"))
                });
                None
            }
        };
        Self { handle, thread }
    }
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    pub fn join(mut self) {
        self.handle.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Active {
    row: Row,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<Result<download::Download, String>>,
}
async fn run(
    mut store: Store,
    mut rx: mpsc::Receiver<Message>,
    state: watch::Sender<State>,
    cancel: CancellationToken,
    ready: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut client: Option<reqwest::Client> = None;
    let mut collections = collections::synchronize(&store)?;
    let (_, settings) = store.settings()?;
    let archived = store.archived_count()?;
    state.send_modify(|s| s.archived_metadata = archived);
    state.send_modify(|s| s.automatic = settings.automatic);
    let mut active: Option<Active> = None;
    let mut probe: Option<probe::Active> = None;
    let mut queue = std::collections::VecDeque::<(u64, bool)>::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            result=async{match &mut probe{Some(job)=>Some((&mut job.task).await),None=>std::future::pending().await}}=>{
                let job=probe.take().unwrap();
                let result=result.unwrap().map_err(|e|format!("URL test stopped: {e}")).and_then(|r|r).map(Reply::UrlTest);
                let _=job.reply.send(result);
            },
            _=cancel.cancelled()=>{if let Some(job)=active.take(){job.cancel.cancel();job.task.abort();let _=store.failed(&job.row,"Application stopped during download",true);}break;},
            result=async{match &mut active{Some(job)=>Some((&mut job.task).await),None=>std::future::pending().await}}=>{
                let job=active.take().unwrap();let outcome=if job.cancel.is_cancelled(){Err("Cancelled".into())}else{result.unwrap().unwrap_or_else(|e|Err(format!("Download task stopped: {e}")))};
                let current=store.get(job.row.id)?;
                if current.revision==job.row.revision&&!current.removed {
                    let(_,settings)=store.settings()?;
                    match outcome {
                        Ok(download::Download::Unchanged)=>{let(interval,_,_)=effective(&current.spec,&settings)?;store.success(&current,None,current.bytes,(&current.etag,&current.modified),interval,true)?;},
                        Ok(download::Download::Updated{file,bytes,etag,modified})=>{
                            if store.used()?.saturating_sub(store.replacing_bytes(&current)?).saturating_add(bytes)>settings.disk_bytes{store.failed(&current,"Published-list disk budget would be exceeded",false)?;}
                            else {
                                let obsolete=store.unreferenced_files(&current)?;let path=store.data_path(&current);std::fs::create_dir_all(path.parent().unwrap()).map_err(|e|e.to_string())?;
                                file.persist(&path).map_err(|e|format!("Cannot publish validated list: {}",e.error))?;
                                let(interval,_,_)=effective(&current.spec,&settings)?;store.success(&current,Some(&path),bytes,(&etag,&modified),interval,false)?;
                                collections=collections::synchronize(&store).map_err(|e|format!("List was published; collection state needs recovery: {e}"))?;
                                for old in obsolete {if Path::new(&old)!=path && !store.referenced(&old)?{std::fs::remove_file(old).map_err(|e|format!("Published list saved; previous owned filename could not be removed: {e}"))?;}}
                            }
                        },
                        Err(error)=>store.failed(&current,&error,job.cancel.is_cancelled())?,
                    }
                }
                state.send_modify(|s|{s.active=None;s.revision+=1;});
            },
            message=rx.recv()=>{
                let Some(message)=message else{break;};
                if let Request::TestUrl(input)=message.request {
                    // A cancelled owner must not make its immediate replacement
                    // look busy. Reap that task before allocating another probe.
                    if probe.as_ref().is_some_and(|job|job.cancel.is_cancelled()) {
                        let job=probe.take().unwrap();job.task.abort();let _=job.task.await;
                        let _=job.reply.send(Err("URL test cancelled".into()));
                    }
                    let error=if probe.is_some(){Some("A URL test is already running; cancel it or wait".to_owned())}else if input.cancel.is_cancelled(){Some("URL test cancelled".to_owned())}else{probe::validate_url(&input.url).err()};
                    if let Some(error)=error{let _=message.reply.send(Err(error));continue;}
                    let token=input.cancel.clone();
                    let agent=store.settings()?.1.user_agent;
                    probe=Some(probe::Active{cancel:token,task:tokio::spawn(probe::fetch(input,agent)),reply:message.reply});
                    continue;
                }
                let message=match message.request {
                    Request::SaveChecked{previous,spec,settings_revision}=>{
                        if store.settings()?.0!=settings_revision {let _=message.reply.send(Err("Subscription defaults changed; reopen the editor before saving".into()));continue;}
                        Message {request:Request::Save{previous,spec},reply:message.reply}
                    },
                    request=>Message{request,reply:message.reply},
                };
                let rebuild=matches!(&message.request,Request::Save{..}|Request::Remove{..}|Request::ApplyBulk(_)|Request::ApplyImport(_));
                let mutating=matches!(&message.request,Request::Save{..}|Request::Remove{..}|Request::ApplyBulk(_)|Request::ApplyImport(_)|Request::SaveSettings{..}|Request::Runtime(_)|Request::Refresh{..}|Request::Cancel);
                let result=(||->Result<Reply,String>{match message.request {
                    Request::TestUrl(_)=>unreachable!("Handled above"),
                    Request::SaveChecked{..}=>unreachable!("Checked above"),
                    Request::ReviewImport{path,replace,defaults,catalogue}=>{let document=transfer::read(&path,&store.settings()?.1)?;store.import_review(document,replace,defaults,catalogue).map(|r|Reply::ImportReview(Box::new(r)))},
                    Request::ApplyImport(review)=>{let (changed,archived)=store.apply_import(&review)?;state.send_modify(|s|s.archived_metadata=archived);if let Some(job)=active.as_ref().filter(|job|changed.contains(&job.row.id)||review.defaults){job.cancel.cancel();}queue.retain(|(id,_)|!changed.contains(id));Ok(Reply::Imported(changed.len()))},
                    Request::Export{path,scope,encoding}=>{let document=store.export_document(&scope)?;transfer::write(&path,&document,encoding).map(Reply::Exported)},
                    Request::ReviewBulk{scope,patch}=>store.bulk_review(&scope,&patch).map(Reply::BulkReview),
                    Request::ApplyBulk(review)=>{let count=store.apply_bulk(&review)?;if let Some(job)=active.as_ref().filter(|job|review.entries.iter().any(|e|e.id==job.row.id)){job.cancel.cancel();}queue.retain(|(id,_)|!review.entries.iter().any(|e|e.id==*id));Ok(Reply::BulkApplied(count))},
                    Request::Collection{group,format}=>collections.iter().find(|c|c.group==group&&c.format.directory()==format.directory()).cloned().map(|c|Reply::Collection(Box::new(c))).ok_or_else(||"Collection membership changed; refresh the collection list".into()),
                    Request::Collections(q)=>collections::page(&collections,&q).map(Reply::Collections),
                    Request::Page(q)=>store.page(&q).map(Reply::Page),Request::Get(id)=>store.get(id).map(|r|Reply::Row(Box::new(r))),
                    Request::Settings=>store.settings().map(|(r,s)|Reply::Settings(r,s)),
                    Request::Save{previous,spec}=>{let saved=store.save(previous,&spec)?;if let Some((id,_))=previous{if let Some(job)=active.as_ref().filter(|a|a.row.id==id){job.cancel.cancel();}queue.retain(|(v,_)|*v!=id);}Ok(Reply::Saved(saved))},
                    Request::Remove{id,revision}=>{if let Some(job)=active.as_ref().filter(|a|a.row.id==id){job.cancel.cancel();}queue.retain(|(v,_)|*v!=id);store.remove(id,revision)?;Ok(Reply::Done)},
                    Request::SaveSettings{revision,settings}=>{if settings.disk_bytes<store.used()?{return Err("Disk budget is below retained downloaded data".into());}store.save_settings(revision,&settings)?;if let Some(job)=&active{job.cancel.cancel();}Ok(Reply::Done)},
                    Request::Runtime(enabled)=>{let(revision,mut config)=store.settings()?;config.automatic=enabled;store.save_settings(revision,&config)?;if !enabled{queue.clear();if let Some(job)=&active{job.cancel.cancel();}}state.send_modify(|s|s.automatic=enabled);Ok(Reply::Done)},
                    Request::Refresh{id,force}=>{let row=store.get(id)?;if row.removed{return Err("Restore this subscription before refreshing".into());}
                    if active.as_ref().is_some_and(|a|a.row.id==id)||queue.iter().any(|(v,_)|*v==id){return Err("This subscription is already downloading or queued".into());}
                    if queue.len()>=MAX_SUBSCRIPTIONS{return Err("Refresh queue is full".into());}queue.push_back((id,force));Ok(Reply::Done)},
                    Request::Cancel=>{queue.clear();if let Some(job)=&active{job.cancel.cancel();}Ok(Reply::Done)},
                    Request::Logs(id)=>store.logs(id).map(Reply::Logs),
                    Request::Preview(id)=>{use std::io::Read;let row=store.get(id)?;let path=row.file.ok_or("No downloaded list yet")?;let file=std::fs::File::open(path).map_err(|e|e.to_string())?;let mut bytes=Vec::new();file.take(64*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;let truncated=bytes.len()>64*1024;bytes.truncate(64*1024);let mut text=String::from_utf8_lossy(&bytes).into_owned();if truncated{text.push_str("\n… preview limited to 64 KiB");}Ok(Reply::Preview(text))},
                }})();
                if result.is_ok()&&rebuild {collections=collections::synchronize(&store).map_err(|e|format!("Subscription metadata saved; collection state needs recovery: {e}"))?;}
                if result.is_ok()&&mutating{state.send_modify(|s|s.revision+=1);}let _=message.reply.send(result);
            },
            _=tick.tick()=>{},
        }
        let local = ready.load(Ordering::Relaxed);
        if state.borrow().local_ready != local {
            state.send_modify(|s| {
                s.local_ready = local;
                s.revision += 1;
            });
        }
        if active.is_none() {
            let (_, settings) = store.settings()?;
            if state.borrow().automatic != settings.automatic {
                state.send_modify(|s| {
                    s.automatic = settings.automatic;
                    s.revision += 1;
                });
            }
            let next = queue.pop_front().or_else(|| {
                if settings.automatic && local {
                    store.due(now()).ok().flatten().map(|id| (id, false))
                } else {
                    None
                }
            });
            if let Some((id, force)) = next {
                let row = store.get(id)?;
                if row.removed {
                    continue;
                }
                let bytes = Arc::new(AtomicU64::new(0));
                let job_cancel = CancellationToken::new();
                store.mark_started(id)?;
                let client = match &client {
                    Some(client) => client.clone(),
                    None => {
                        let value = reqwest::Client::builder()
                            .pool_max_idle_per_host(1)
                            .connect_timeout(Duration::from_secs(10))
                            .redirect(reqwest::redirect::Policy::limited(5))
                            .build()
                            .map_err(|e| e.to_string())?;
                        client = Some(value.clone());
                        value
                    }
                };
                let tmp = store.root.join("tmp");
                let spec = row.clone();
                let token = job_cancel.clone();
                let progress = bytes.clone();
                let task = tokio::spawn(async move {
                    download::fetch(client, spec, settings, &tmp, force, token, progress).await
                });
                active = Some(Active {
                    row,
                    cancel: job_cancel,
                    task,
                });
                state.send_modify(|s| {
                    s.active = Some(id);
                    s.downloaded = bytes;
                    s.revision += 1;
                });
            }
        }
    }
    if let Some(job) = probe.take() {
        job.cancel.cancel();
        job.task.abort();
        let _ = job.reply.send(Err("URL test cancelled".into()));
    }
    Ok(())
}
