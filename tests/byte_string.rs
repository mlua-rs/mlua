use bstr::{BStr, BString};
use rlua::Result;

include!("_lua.rs");

#[test]
fn byte_string_round_trip() -> Result<()> {
    let lua = make_lua();

    lua.load(
        r#"
        invalid_sequence_identifier = "\xa0\xa1"
        invalid_2_octet_sequence_2nd = "\xc3\x28"
        invalid_3_octet_sequence_2nd = "\xe2\x28\xa1"
        invalid_3_octet_sequence_3rd = "\xe2\x82\x28"
        invalid_4_octet_sequence_2nd = "\xf0\x28\x8c\xbc"
        invalid_4_octet_sequence_3rd = "\xf0\x90\x28\xbc"
        invalid_4_octet_sequence_4th = "\xf0\x28\x8c\x28"

        an_actual_string = "Hello, world!"
    "#,
    )
    .exec()?;

    let globals = lua.globals();

    let isi = globals.get::<_, BString>("invalid_sequence_identifier")?;
    assert_eq!(isi, [0xa0, 0xa1].as_ref());

    let i2os2 = globals.get::<_, BString>("invalid_2_octet_sequence_2nd")?;
    assert_eq!(i2os2, [0xc3, 0x28].as_ref());

    let i3os2 = globals.get::<_, BString>("invalid_3_octet_sequence_2nd")?;
    assert_eq!(i3os2, [0xe2, 0x28, 0xa1].as_ref());

    let i3os3 = globals.get::<_, BString>("invalid_3_octet_sequence_3rd")?;
    assert_eq!(i3os3, [0xe2, 0x82, 0x28].as_ref());

    let i4os2 = globals.get::<_, BString>("invalid_4_octet_sequence_2nd")?;
    assert_eq!(i4os2, [0xf0, 0x28, 0x8c, 0xbc].as_ref());

    let i4os3 = globals.get::<_, BString>("invalid_4_octet_sequence_3rd")?;
    assert_eq!(i4os3, [0xf0, 0x90, 0x28, 0xbc].as_ref());

    let i4os4 = globals.get::<_, BString>("invalid_4_octet_sequence_4th")?;
    assert_eq!(i4os4, [0xf0, 0x28, 0x8c, 0x28].as_ref());

    let aas = globals.get::<_, BString>("an_actual_string")?;
    assert_eq!(aas, b"Hello, world!".as_ref());

    globals.set::<_, &BStr>("bstr_invalid_sequence_identifier", isi.as_ref())?;
    globals.set::<_, &BStr>("bstr_invalid_2_octet_sequence_2nd", i2os2.as_ref())?;
    globals.set::<_, &BStr>("bstr_invalid_3_octet_sequence_2nd", i3os2.as_ref())?;
    globals.set::<_, &BStr>("bstr_invalid_3_octet_sequence_3rd", i3os3.as_ref())?;
    globals.set::<_, &BStr>("bstr_invalid_4_octet_sequence_2nd", i4os2.as_ref())?;
    globals.set::<_, &BStr>("bstr_invalid_4_octet_sequence_3rd", i4os3.as_ref())?;
    globals.set::<_, &BStr>("bstr_invalid_4_octet_sequence_4th", i4os4.as_ref())?;
    globals.set::<_, &BStr>("bstr_an_actual_string", aas.as_ref())?;

    lua.load(
        r#"
        assert(bstr_invalid_sequence_identifier == invalid_sequence_identifier)
        assert(bstr_invalid_2_octet_sequence_2nd == invalid_2_octet_sequence_2nd)
        assert(bstr_invalid_3_octet_sequence_2nd == invalid_3_octet_sequence_2nd)
        assert(bstr_invalid_3_octet_sequence_3rd == invalid_3_octet_sequence_3rd)
        assert(bstr_invalid_4_octet_sequence_2nd == invalid_4_octet_sequence_2nd)
        assert(bstr_invalid_4_octet_sequence_3rd == invalid_4_octet_sequence_3rd)
        assert(bstr_invalid_4_octet_sequence_4th == invalid_4_octet_sequence_4th)
        assert(bstr_an_actual_string == an_actual_string)
    "#,
    )
    .exec()?;

    globals.set::<_, BString>("bstring_invalid_sequence_identifier", isi)?;
    globals.set::<_, BString>("bstring_invalid_2_octet_sequence_2nd", i2os2)?;
    globals.set::<_, BString>("bstring_invalid_3_octet_sequence_2nd", i3os2)?;
    globals.set::<_, BString>("bstring_invalid_3_octet_sequence_3rd", i3os3)?;
    globals.set::<_, BString>("bstring_invalid_4_octet_sequence_2nd", i4os2)?;
    globals.set::<_, BString>("bstring_invalid_4_octet_sequence_3rd", i4os3)?;
    globals.set::<_, BString>("bstring_invalid_4_octet_sequence_4th", i4os4)?;
    globals.set::<_, BString>("bstring_an_actual_string", aas)?;

    lua.load(
        r#"
        assert(bstring_invalid_sequence_identifier == invalid_sequence_identifier)
        assert(bstring_invalid_2_octet_sequence_2nd == invalid_2_octet_sequence_2nd)
        assert(bstring_invalid_3_octet_sequence_2nd == invalid_3_octet_sequence_2nd)
        assert(bstring_invalid_3_octet_sequence_3rd == invalid_3_octet_sequence_3rd)
        assert(bstring_invalid_4_octet_sequence_2nd == invalid_4_octet_sequence_2nd)
        assert(bstring_invalid_4_octet_sequence_3rd == invalid_4_octet_sequence_3rd)
        assert(bstring_invalid_4_octet_sequence_4th == invalid_4_octet_sequence_4th)
        assert(bstring_an_actual_string == an_actual_string)
    "#,
    )
    .exec()?;

    Ok(())
}
