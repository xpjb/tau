//! Byte-bounded verified body cache. Metadata/cursors are independent of body
//! residency; eviction never claims absent bytes or removes authored outbox data.
use super::*;

pub const DEFAULT_CACHE_BYTES:u64=512*1024*1024;
pub fn stored_bytes(db:&Connection,scope:&str,id:&str)->Result<u64> {
    Ok(db.query_row("SELECT coalesce(sum(length(c.data)),0) FROM block_parts p JOIN block_chunks c ON c.hash=p.hash WHERE p.scope=?1 AND p.id=?2",params![scope,id],|r|r.get(0))?)
}
pub fn enforce(db:&Connection,scope:&str,id:&str,limit:u64)->Result<()> {
    writing(db)?;
    let clock:u64=db.query_row("UPDATE block_usage SET clock=clock+1 WHERE singleton=1 RETURNING clock",[],|r|r.get(0))?;
    db.execute("INSERT INTO block_cache_access(scope,id,touched) VALUES(?1,?2,?3) ON CONFLICT(scope,id) DO UPDATE SET touched=excluded.touched",params![scope,id,clock])?;
    loop {
        let bytes:u64=db.query_row("SELECT bytes FROM block_usage WHERE singleton=1",[],|r|r.get(0))?;
        if bytes<=limit {return Ok(());}
        let victim=db.query_row("SELECT p.scope,p.id FROM block_parts p LEFT JOIN block_cache_access a ON a.scope=p.scope AND a.id=p.id
            WHERE p.scope!=?1 OR p.id!=?2 GROUP BY p.scope,p.id ORDER BY coalesce(a.touched,0),p.scope,p.id LIMIT 1",params![scope,id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
        let (scope,id)=victim.context("Active block exceeds cache quota")?;
        db.execute("DELETE FROM block_parts WHERE scope=?1 AND id=?2",params![scope,id])?;
        db.execute("DELETE FROM block_cache_access WHERE scope=?1 AND id=?2",params![scope,id])?;
    }
}
