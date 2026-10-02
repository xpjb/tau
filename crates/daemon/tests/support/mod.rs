//! Real test client: bounded control plus explicit native reads.
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value,json};
use tau_net::blocks::*;
use tau_net::native::{Client as DataClient, Priority, Update};
use tokio_tungstenite::{connect_async,tungstenite::{Message,client::IntoClientRequest}};

pub struct Client {
    pub socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    pub seen: Vec<Value>,
    pub data: DataClient,
    pub lineage: String,
}
impl Client {
    #[allow(dead_code)]
    pub async fn connect(url: &str) -> Self {Self::connect_token(url,"isolated-test-token").await}
    pub async fn connect_token(url: &str, token: &str) -> Self {
        let mut request=url.into_client_request().unwrap();
        request.headers_mut().insert("authorization",format!("Bearer {token}").parse().unwrap());
        let (socket,_)=connect_async(request).await.unwrap();
        let mut client=Self {socket,seen:vec![],data:DataClient::bind().await.unwrap(),lineage:String::new()};
        let hello=client.until(|m|m["type"]=="hello").await;
        assert_eq!(hello["protocolVersion"],tau_net::PROTOCOL_VERSION);
        client.request(json!({"id":"data","type":"connect_blocks","nodeId":client.data.node_id()})).await;
        client.request(json!({"id":"initial-head","type":"list_sessions"})).await;
        client
    }
    pub async fn until(&mut self, predicate: impl Fn(&Value)->bool) -> Value {
        tokio::time::timeout(Duration::from_secs(15),async {
            loop {
                match self.socket.next().await.unwrap().unwrap() {
                    Message::Text(text)=>{
                        assert!(text.len()<=tau_net::MAX_CONTROL_BYTES);
                        let mut value:Value=serde_json::from_str(&text).unwrap();
                        assert!(!matches!(value["type"].as_str(),Some("transcript_update"|"transcript_snapshot"|"transcript_page")));
                        if value["type"]=="block_connection" {
                            let offer:BulkOffer=serde_json::from_value(value["offer"].clone()).unwrap();
                            self.lineage=offer.lineage.clone();self.data.configure(&offer,"127.0.0.1").await.unwrap();
                        }
                        if value["type"]=="data" {
                            let reference:ContentRef=serde_json::from_value(value["content"].clone()).unwrap();
                            assert_eq!(reference.lineage,self.lineage);
                            let bytes=self.body(&reference.scope,&reference.id).await;
                            assert_eq!(blake3::hash(&bytes).to_hex().as_str(),reference.hash);
                            value=serde_json::from_slice(&bytes).unwrap();
                        }
                        if value["type"]=="resync_required" && value["sessionId"].is_null() {
                            self.socket.send(Message::Text(json!({"id":"head-sync","type":"list_sessions"}).to_string().into())).await.unwrap();
                        }
                        if value["type"]=="session_page" {assert!(value["next"].is_null(),"Use the paged frontend for large-catalogue scenarios");value["type"]=json!("sessions");}
                        if value["type"]=="project_page" {assert!(value["next"].is_null());value["type"]=json!("projects");}
                        self.seen.push(value.clone());if predicate(&value) {return value;}
                    }
                    Message::Ping(_)=>self.socket.flush().await.unwrap(),
                    other=>panic!("Unexpected socket event {other:?}"),
                }
            }
        }).await.expect("Expected daemon control event")
    }
    pub async fn request(&mut self, value:Value)->Value {
        let id=value["id"].clone();let bytes=serde_json::to_vec(&value).unwrap();
        let value=if bytes.len()>tau_net::MAX_CONTROL_BYTES {
            let hash=blake3::hash(&bytes).to_hex().to_string();
            let spec=UploadSpec {id:hash.clone(),length:bytes.len() as u64,hash:hash.clone(),purpose:UploadPurpose::Command};
            let mut upload=self.data.uploader(spec).await.unwrap();
            upload.copy_from(&mut std::io::Cursor::new(&bytes)).await.unwrap();
            upload.finish().await.unwrap();
            json!({"id":id,"type":"input","content":ContentRef {lineage:self.lineage.clone(),scope:UPLOAD_SCOPE.into(),id:hash.clone(),length:bytes.len() as u64,hash}})
        } else {value};
        self.socket.send(Message::Text(value.to_string().into())).await.unwrap();
        self.until(|m|m["type"]=="response" && m["requestId"]==id).await
    }
    pub async fn body(&self,scope:&str,id:&str)->Vec<u8> {
        let mut reader=self.data.read(BlockWatch::Block(BlockRequest {scope:scope.into(),id:id.into(),version:0,offset:0,follow:false}),Priority::Foreground).await.unwrap();
        let mut bytes=vec![];
        while let Some(update)=reader.next().await.unwrap() {match update {
            Update::Range {range,..}=>{assert_eq!(range.offset,bytes.len() as u64);bytes.extend(range.bytes);}
            Update::Block {..}=>{},other=>panic!("Unexpected body update {other:?}"),
        }}bytes
    }
    async fn directory(&self,scope:&str,parent:Option<String>,before:Option<FeedPosition>)->FeedPage {
        let mut reader=self.data.read(BlockWatch::Feed(FeedRequest {scope:scope.into(),parent,cursor:None,floor:0,before}),Priority::Foreground).await.unwrap();
        let Some(Update::Page {page,..})=reader.next().await.unwrap() else {panic!("Expected directory page");};page
    }
    pub async fn page(&self,id:&str,before:Option<FeedPosition>)->Value {
        let page=self.directory(id,None,before).await;
        let mut events=vec![];let mut queue=tau_net::QueueState::native();
        for record in page.records {
            let BlockRecord::Put {block:h}=record else {continue;};
            if h.id=="@queue" {queue=serde_json::from_slice(&self.body(id,"@queue").await).unwrap();}
            if let Some(value)=h.meta.get("event") {
                let mut event:tau_net::Event=serde_json::from_value(value.clone()).unwrap();
                event.text=if h.kind==BlockKind::Tool {String::new()} else {String::from_utf8(self.body(id,&h.id).await).unwrap()};
                events.push(event);
            }
            if h.kind==BlockKind::Tool || h.id=="@queue" {
                let mut before=None;
                loop {
                    let children=self.directory(id,Some(h.id.clone()),before).await;
                    for child in children.records {
                        let BlockRecord::Put {block:c}=child else {continue;};
                        if c.meta.get("inputFor").is_some() {events.last_mut().unwrap().text=String::from_utf8(self.body(id,&c.id).await).unwrap();}
                        else if let Some(value)=c.meta.get("event") {
                            let mut event:tau_net::Event=serde_json::from_value(value.clone()).unwrap();
                            event.text=String::from_utf8(self.body(id,&c.id).await).unwrap();events.push(event);
                        } else if let Some(value)=c.meta.get("request") {
                            let mut request:tau_net::QueuedRequest=serde_json::from_value(value.clone()).unwrap();
                            request.text=String::from_utf8(self.body(id,&c.id).await).unwrap();queue.requests.push(request);
                        }
                    }
                    before=children.before;if before.is_none() {break;}
                }
            }
        }
        events.sort_by_key(|e|e.order);
        json!({"generation":format!("{}:{id}",page.cursor.lineage),"sequence":page.cursor.sequence,"events":events,"queue":queue,"before":page.before,"delivered":[]})
    }
    pub async fn open(&mut self,id:&str)->Value {
        assert_eq!(self.request(json!({"id":format!("open-{}",uuid::Uuid::new_v4()),"type":"get_session","sessionId":id})).await["ok"],true);
        self.page(id,None).await
    }
}
