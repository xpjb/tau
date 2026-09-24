//! Real protocol-17 test client: bounded control plus explicit native reads.
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value,json};
use tau_blocks::*;
use tau_transfer::blocks::{Client as DataClient, Header};
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
        assert_eq!(hello["protocolVersion"],tau_protocol::PROTOCOL_VERSION);
        client.request(json!({"id":"data","type":"connect_blocks","nodeId":client.data.node_id()})).await;
        client
    }
    pub async fn until(&mut self, predicate: impl Fn(&Value)->bool) -> Value {
        tokio::time::timeout(Duration::from_secs(15),async {
            loop {
                match self.socket.next().await.unwrap().unwrap() {
                    Message::Text(text)=>{
                        assert!(text.len()<=tau_protocol::MAX_CONTROL_BYTES);
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
        let value=if bytes.len()>tau_protocol::MAX_CONTROL_BYTES {
            let hash=blake3::hash(&bytes).to_hex().to_string();
            let spec=UploadSpec {id:hash.clone(),length:bytes.len() as u64,hash:hash.clone(),purpose:UploadPurpose::Command};
            let mut upload=self.data.uploader(spec).await.unwrap();
            for chunk in bytes[upload.status.offset as usize..].chunks(BLOCK_CHUNK_BYTES) {upload.write(chunk).await.unwrap();}
            upload.finish().await.unwrap();
            json!({"id":id,"type":"input","content":ContentRef {lineage:self.lineage.clone(),scope:UPLOAD_SCOPE.into(),id:hash.clone(),length:bytes.len() as u64,hash}})
        } else {value};
        self.socket.send(Message::Text(value.to_string().into())).await.unwrap();
        self.until(|m|m["type"]=="response" && m["requestId"]==id).await
    }
    pub async fn body(&self,scope:&str,id:&str)->Vec<u8> {
        let mut watch=self.data.watch(BlockWatch::Block(BlockRequest {scope:scope.into(),id:id.into(),version:0,offset:0,follow:false})).await.unwrap();
        let mut bytes=vec![];
        loop {let (frame,n)=watch.next().await.unwrap();match &frame.header {
            Header::Data {offset,..}=>{assert_eq!(*offset,bytes.len() as u64);bytes.extend(frame.decoded().unwrap());}
            Header::End=>return bytes,
            Header::Error {message}=>panic!("{message}"),
            Header::Block {..}=>{},
            other=>panic!("Unexpected body frame {other:?}"),
        }let _=watch.consumed(n).await;}
    }
    async fn directory(&self,scope:&str,parent:Option<String>,before:Option<FeedPosition>)->FeedPage {
        let mut watch=self.data.watch(BlockWatch::Feed(FeedRequest {scope:scope.into(),parent,cursor:None,floor:0,before})).await.unwrap();
        let mut records=vec![];
        loop {let (frame,n)=watch.next().await.unwrap();match frame.header {
            Header::Record {record,..}=>records.push(record),
            Header::Page {reset,cursor,floor,before,more,..}=>return FeedPage {reset,cursor,floor,before,more,records},
            Header::Error {message}=>panic!("{message}"),
            other=>panic!("Unexpected directory frame {other:?}"),
        }let _=watch.consumed(n).await;}
    }
    pub async fn page(&self,id:&str,before:Option<FeedPosition>)->Value {
        let page=self.directory(id,None,before).await;
        let mut events=vec![];let mut queue=tau_protocol::QueueState::native();
        for record in page.records {
            let BlockRecord::Put {block:h}=record else {continue;};
            if h.id=="@queue" {queue=serde_json::from_slice(&self.body(id,"@queue").await).unwrap();}
            if let Some(value)=h.meta.get("event") {
                let mut event:tau_protocol::Event=serde_json::from_value(value.clone()).unwrap();
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
                            let mut event:tau_protocol::Event=serde_json::from_value(value.clone()).unwrap();
                            event.text=String::from_utf8(self.body(id,&c.id).await).unwrap();events.push(event);
                        } else if let Some(value)=c.meta.get("request") {
                            let mut request:tau_protocol::QueuedRequest=serde_json::from_value(value.clone()).unwrap();
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
