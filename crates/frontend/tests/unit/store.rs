#[test]
fn kernel_enospc_aborts_the_bounded_import_copy() {
    let mut input=std::io::Cursor::new(vec![5;32768]);let mut full=std::fs::OpenOptions::new().write(true).open("/dev/full").unwrap();
    let error=super::copy_bytes(&mut input,&mut full,50000).unwrap_err();assert!(format!("{error:#}").contains("No space left"));
}
