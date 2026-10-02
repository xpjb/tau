//! Verified application reads. Framing, credit, decompression, batching and
//! fair stream renewal are private to transport; consumers only apply updates.
use super::*;

#[derive(Clone, Copy)]
pub enum Priority { Foreground, Background, Descriptor }

#[derive(Debug)]
pub enum Update {
    Page { request: FeedRequest, page: FeedPage },
    Block { scope: String, block: BlockHeader },
    Range { scope: String, range: ContentRange },
    /// The requested identity disappeared before its read. This is not a
    /// tombstone/cursor certificate: never delete replica state on this hint.
    Absent { scope: String, id: String },
}

/// Apply each returned update before asking for the next one. That call returns
/// credit and advances from the applied checkpoint. Drop cancels only this read.
/// Scheduled slices are renewed here, without replaying an applied prefix/page.
pub struct Reader {
    peer: Peer,
    connection: Connection,
    stream: Option<Watcher>,
    request: BlockWatch,
    priority: Priority,
    records: Vec<Vec<BlockRecord>>,
    head: Option<BlockHeader>,
    credit: u32,
    received: u64,
    complete: bool,
}
impl Peer {
    pub async fn read(&self, request: BlockWatch, priority: Priority) -> Result<Reader> {
        let count=match &request {BlockWatch::Feed(_)=>1,BlockWatch::Feeds {requests}=>requests.len(),BlockWatch::Block(_)=>0};
        ensure!(count<=16 && !matches!(&request,BlockWatch::Feeds {requests} if requests.is_empty()),"Invalid feed batch");
        let (stream,connection)=self.reader_stream(request.clone(),priority,None).await?;
        Ok(Reader {peer:self.clone(),connection,stream:Some(stream),request,priority,records:vec![vec![];count],head:None,credit:0,received:0,complete:false})
    }
    async fn reader_stream(&self, request:BlockWatch, priority:Priority, connection:Option<Connection>) -> Result<(Watcher,Connection)> {
        self.during(async {
            let metadata=matches!(request,BlockWatch::Feed(_)|BlockWatch::Feeds {..});
            let (class,priority)=match priority {
                Priority::Background=>(&self.transport.bulk,-10), Priority::Descriptor=>(&self.transport.descriptors,8),
                Priority::Foreground if metadata=>(&self.transport.metadata,10), Priority::Foreground=>(&self.transport.foreground,5),
            };
            let class=class.clone().acquire_owned().await?;
            let permit=self.transport.streams.clone().acquire_owned().await?;
            // Renew on the original authenticated connection, never a newly
            // configured peer/source. Connection recovery belongs to the owner.
            let connection=match connection {Some(connection)=>connection,None=>self.connection().await?};
            let (mut send,recv)=connection.open_bi().await?;
            let offset=if let BlockWatch::Block(block)=&request {block.offset} else {0};
            let bytes=encode(&Frame::metadata(Header::Watch {request,credit:BLOCK_WINDOW_BYTES,priority}))?;
            send.write_all(&bytes).await?;self.transport.stats.tx.fetch_add(bytes.len() as u64,Ordering::Relaxed);
            self.transport.stats.opened();self.transport.stats.resumed.fetch_add(offset,Ordering::Relaxed);
            Ok((Watcher {stats:self.transport.stats.clone(),complete:false,send,recv,_permit:permit,_class:class},connection))
        }).await
    }
}
impl Reader {
    pub fn received_bytes(&self) -> u64 { self.received }
    pub async fn next(&mut self) -> Result<Option<Update>> {
        if self.complete {return Ok(None);}
        self.peer.current()?;
        loop {
            if self.stream.is_none() {self.stream=Some(self.peer.reader_stream(self.request.clone(),self.priority,Some(self.connection.clone())).await?.0);}
            let stream=self.stream.as_mut().unwrap();
            // The peer may already have finished its credit half after writing
            // End/Yield. Read-side completion, not that final ACK, is authoritative.
            if self.credit>0 {let _=stream.consumed(std::mem::take(&mut self.credit)).await;}
            let (frame,bytes)=stream.next().await?;
            self.received+=bytes as u64;self.credit=bytes;
            match frame.header {
                Header::Record {watch,record}=>{
                    let records=self.records.get_mut(watch).context("Unrequested feed")?;
                    ensure!(records.len()<MAX_FEED_PAGE,"Unexpected feed record");records.push(record);
                }
                Header::Page {watch,reset,cursor,floor,before,more}=>{
                    let requests=match &mut self.request {
                        BlockWatch::Feed(request)=>std::slice::from_mut(request),
                        BlockWatch::Feeds {requests}=>requests.as_mut_slice(),
                        _=>bail!("Unrequested feed page"),
                    };
                    let request=requests.get_mut(watch).context("Unrequested feed")?;
                    let original=request.clone();request.cursor=Some(cursor.clone());request.floor=floor;
                    let page=FeedPage {reset,cursor,floor,before,more,records:std::mem::take(&mut self.records[watch])};
                    return Ok(Some(Update::Page {request:original,page}));
                }
                Header::Block {block}=>{
                    let BlockWatch::Block(request)=&mut self.request else {bail!("Unexpected block header");};
                    ensure!(block.id==request.id && block.length<=MAX_BLOCK_BYTES,"Unrequested or oversized block");
                    if request.version!=block.version {request.version=block.version;request.offset=0;}
                    ensure!(request.offset<=block.length,"Block head precedes its applied prefix");
                    self.head=Some(block.clone());
                    return Ok(Some(Update::Block {scope:request.scope.clone(),block}));
                }
                Header::Data {version,offset,..}=>{
                    let BlockWatch::Block(request)=&mut self.request else {bail!("Unrequested body");};
                    let header=self.head.as_ref().context("Content without a requested header")?;
                    ensure!(header.version==version && request.version==version && request.offset==offset,"Unexpected content version/offset");
                    let bytes=decode(&frame,&self.peer.transport.stats)?;
                    ensure!(!bytes.is_empty() && offset+bytes.len() as u64<=header.length,"Content exceeds its declared head");
                    request.offset+=bytes.len() as u64;
                    let Header::Data {hash,..}=frame.header else {unreachable!()};
                    return Ok(Some(Update::Range {scope:request.scope.clone(),range:ContentRange {header:header.clone(),offset,hash,bytes}}));
                }
                Header::Absent=>{
                    let BlockWatch::Block(request)=&self.request else {bail!("Unexpected absent feed");};
                    self.complete=true;self.stream=None;self.credit=0;
                    return Ok(Some(Update::Absent {scope:request.scope.clone(),id:request.id.clone()}));
                }
                Header::End | Header::Yield=>{
                    ensure!(self.records.iter().all(Vec::is_empty),"Watch ended before its metadata checkpoint");
                    let end=matches!(frame.header,Header::End);
                    if end && let BlockWatch::Block(request)=&self.request {
                        let head=self.head.as_ref().context("Body ended without a header")?;
                        ensure!(request.offset==head.length && (!request.follow || head.sealed),"Body ended before its declared head");
                    }
                    self.stream=None;self.credit=0;self.head=None;
                    if end {self.complete=true;return Ok(None);}
                    // Releasing class/global permits is part of each checkpoint,
                    // including quiet feeds and slow but completing responses.
                }
                Header::Error {message}=>bail!("{message}"),
                _=>bail!("Unexpected read response"),
            }
        }
    }
}

pub(super) fn decode(frame:&Frame,stats:&Counters)->Result<Vec<u8>> {
    match frame.decoded() {
        Ok(bytes)=>{stats.content_rx.fetch_add(bytes.len() as u64,Ordering::Relaxed);Ok(bytes)}
        Err(error)=>{stats.integrity.fetch_add(1,Ordering::Relaxed);Err(error)}
    }
}
