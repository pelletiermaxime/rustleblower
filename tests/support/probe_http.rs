use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;
pub struct Server {
    pub url: String,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub held: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}
impl Server {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let stop = CancellationToken::new();
        let held = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let (log, token, holding, releasing) = (
            requests.clone(),
            stop.clone(),
            held.clone(),
            release.clone(),
        );
        let task = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _=token.cancelled()=>break,
                    _=children.join_next(), if !children.is_empty()=>{},
                    result=listener.accept()=>{
                        let Ok((mut socket,_))=result else{break};if children.len()>=8 {continue}
                        let (log,token,held,release)=(log.clone(),token.clone(),holding.clone(),releasing.clone());
                        children.spawn(async move {
                            let mut request=Vec::new();let mut buf=[0;1024];
                            loop {let n=match tokio::time::timeout(std::time::Duration::from_secs(2),socket.read(&mut buf)).await {Ok(Ok(n))=>n,_=>return};if n==0{return}request.extend_from_slice(&buf[..n]);if request.windows(4).any(|v|v==b"\r\n\r\n") {break}
if request.len()>8192{return}}
                            let text=String::from_utf8_lossy(&request);let mut words=text.split_whitespace();let method=words.next().unwrap_or("").to_owned();let path=words.next().unwrap_or("/").to_owned();
                            {let mut log=log.lock().unwrap();if log.len()<128 {log.push(format!("{method} {path}"));}}
                            if path=="/slow-head" {held.store(true,Ordering::Release);while !release.load(Ordering::Acquire) && !token.is_cancelled(){tokio::time::sleep(std::time::Duration::from_millis(5)).await;}}
                            if path=="/redirect" || path=="/loop" || path=="/long-redirect" {
                                let location=if path=="/redirect" {"/hosts".to_owned()}else if path=="/loop" {"/loop".to_owned()}else{format!("/{}","x".repeat(2100))};
                                let _=socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await;return;
                            }
                            let status=if path=="/gone" {404}else if method=="HEAD" && path=="/reject-head" {405}else if method=="HEAD" && path=="/forbid-head" {403}else{200};
                            let mut body=if path=="/bad" {"<html>invalid</html>\n".repeat(20)}else if path=="/short" {"<html>short</html>\n".into()}else if path=="/many" {"0.0.0.0 many.example\n".repeat(1000)}else if path=="/long" {"x".repeat(131072)}else{(0..20).map(|i|format!("0.0.0.0 good-{i}.example\n")).collect::<String>()};
                            if path=="/split" {body="0.0.0.0 split.example\n".repeat(20);let _=socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await;if method=="HEAD" {return}for chunk in body.as_bytes().chunks(3) {if socket.write_all(format!("{:x}\r\n",chunk.len()).as_bytes()).await.is_err(){return}
if socket.write_all(chunk).await.is_err(){return}let _=socket.write_all(b"\r\n").await;}let _=socket.write_all(b"0\r\n\r\n").await;return;}
                            if path=="/slow" && method=="GET" {let _=socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n0.0.0.0 pending.example\n").await;held.store(true,Ordering::Release);while !release.load(Ordering::Acquire) && !token.is_cancelled(){tokio::time::sleep(std::time::Duration::from_millis(5)).await;}return;}
                            let length=if path=="/broken" {5000}else{body.len()};
                            let _=socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").as_bytes()).await;
                            if method!="HEAD" {let _=socket.write_all(body.as_bytes()).await;}
                        });
                    }
                }
            }
        });
        Self {
            url,
            requests,
            held,
            release,
            stop,
            task,
        }
    }
    pub fn release(&self) {
        self.release.store(true, Ordering::Release);
    }
}
