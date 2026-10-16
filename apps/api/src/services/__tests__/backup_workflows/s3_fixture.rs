use crate::services::s3_backups::S3Settings;
use axum::{
    body::{to_bytes, Body},
    http::{Request, Response},
    routing::any,
    Router,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
pub(super) struct Reply {
    pub(super) method: &'static str,
    pub(super) status: u16,
    pub(super) headers: Vec<(&'static str, String)>,
    pub(super) body: Vec<u8>,
}
pub(super) fn reply(method: &'static str, status: u16, body: impl Into<Vec<u8>>) -> Reply {
    Reply {
        method,
        status,
        headers: vec![],
        body: body.into(),
    }
}
pub(super) struct Fake {
    pub(super) settings: S3Settings,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    calls: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Fake {
    pub(super) async fn new(name: &str, replies: Vec<Reply>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let settings = S3Settings {
            endpoint: endpoint.clone(),
            region: "fixture".into(),
            bucket: "bucket".into(),
            prefix: "fixture".into(),
            access_key: "synthetic-access".into(),
            secret_key: "synthetic-secret".into(),
            policy: crate::services::s3_transport::S3Policy {
                development_origin: Some(endpoint.parse().unwrap()),
                ..Default::default()
            },
        };
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let queue = replies.clone();
        let log = calls.clone();
        let name = name.to_owned();
        let router=Router::new().fallback(any(move |request:Request<Body>|{
            let queue=queue.clone();let log=log.clone();let name=name.clone();
            async move{
                let (parts,body)=request.into_parts();
                let bytes=to_bytes(body,70*1024*1024).await.unwrap();
                let headers=parts.headers.iter().filter(|(k,_)|k.as_str().starts_with("x-amz-meta-")).map(|(k,v)|(k.to_string(),json!(v.to_str().unwrap()))).collect::<serde_json::Map<_,_>>();
                let query=url::form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes()).into_owned().collect::<std::collections::BTreeMap<_,_>>();
                let mut log=log.lock().unwrap();
                let index=log.len();
                let dir=std::path::PathBuf::from(std::env::var("BACKUP_CONTRACT_ARTIFACT_DIR").unwrap());
                let run=std::env::var("BACKUP_CONTRACT_RUN").unwrap_or_else(|_|"actual".into());
                std::fs::write(dir.join(format!("{run}-s3-{name}-{index}.body")),&bytes).unwrap();
                log.push(json!({"method":parts.method.as_str(),"path":parts.uri.path(),"query":query,"metadata":headers,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(&bytes))}));
                let r=queue.lock().unwrap().pop_front().expect("unexpected S3 request");
                assert_eq!(parts.method,r.method);
                let mut response=Response::builder().status(r.status);
                for (k,v) in r.headers {response=response.header(k,v);}
                response.body(Body::from(r.body)).unwrap()
            }
        }));
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            settings,
            replies,
            calls,
            task,
        }
    }
    pub(super) fn finish(self) -> Value {
        assert!(
            self.replies.lock().unwrap().is_empty(),
            "unconsumed S3 replies"
        );
        self.task.abort();
        json!(self.calls.lock().unwrap().clone())
    }
}
pub(super) fn head(created: &str, checksum: &str) -> Reply {
    Reply {
        method: "HEAD",
        status: 200,
        headers: vec![
            ("x-amz-meta-panda-created-at", created.into()),
            ("x-amz-meta-panda-sha256", checksum.into()),
            ("x-amz-meta-panda-format", "panda-recovery-v3".into()),
        ],
        body: vec![],
    }
}
pub(super) fn error(method: &'static str) -> Reply {
    reply(
        method,
        400,
        b"<Error><Code>InvalidRequest</Code><Message>synthetic rejection</Message></Error>"
            .to_vec(),
    )
}
