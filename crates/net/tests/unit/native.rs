use super::*;

#[test]
fn compression_is_chunk_local_bounded_and_verified_after_decompression() {
    let header = BlockHeader { id:"a".into(),parent:None,order:0,kind:BlockKind::Code,meta:serde_json::json!({}),version:1,length:BLOCK_CHUNK_BYTES as u64,sealed:true,revision:1 };
    let bytes = vec![b'x';BLOCK_CHUNK_BYTES];
    let range = ContentRange { header,offset:0,hash:blake3::hash(&bytes).to_hex().to_string(),bytes:bytes.clone() };
    let mut frame = Frame::data(range.header.version,range.offset,range.hash.clone(),&range.bytes).unwrap();
    assert!(matches!(frame.header,Header::Data {codec:Codec::Zstd,..}));
    assert!(frame.data.len() < 100); assert_eq!(frame.decoded().unwrap(),bytes);
    if let Header::Data { length,.. } = &mut frame.header { *length = (BLOCK_CHUNK_BYTES+1) as u32; }
    assert!(frame.decoded().is_err());
    if let Header::Data { length,hash,.. } = &mut frame.header { *length = BLOCK_CHUNK_BYTES as u32; *hash = "bad".into(); }
    assert!(frame.decoded().is_err());
}

#[test]
fn malformed_compression_is_not_connection_loss() {
    let broken=Frame {header:Header::Data {version:1,offset:0,hash:String::new(),length:32,codec:Codec::Zstd},data:b"not a zstd frame".to_vec()};
    let error=broken.decoded().unwrap_err();assert!(error.is::<std::io::Error>());
    assert!(!is_connection_error(&error),"Decompression IO errors remain genuine content failures");
}
