//! The built program reads a shapefile's attribute table.

use std::process::Command;

#[test]
fn info_and_convert_read_dbf() {
    let d = std::env::temp_dir().join(format!("nd-cli-dbf-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    // One character field "A" of 2, one record.
    let mut f = vec![0u8; 32];
    f[0] = 3;
    f[4] = 1;
    f[8] = 65;
    f[10] = 3;
    let mut field = [0u8; 32];
    field[0] = b'A';
    field[11] = b'C';
    field[16] = 2;
    f.extend(field);
    f.push(0x0D);
    f.extend_from_slice(b" ok\x1a");
    let path = d.join("made up.dbf");
    std::fs::write(&path, f).unwrap();
    let run = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_natendo-cli"))
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{args:?}: {o:?}");
        String::from_utf8(o.stdout).unwrap()
    };
    let p = path.to_str().unwrap();
    assert!(run(&["info", p]).contains("columns:     2"));
    let out = d.join("out.csv");
    run(&["convert", p, "-o", out.to_str().unwrap()]);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "FID,A\r\n0,ok\r\n");
    let _ = std::fs::remove_dir_all(&d);
}
