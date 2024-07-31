use bstr::{BStr, BString};
use mlua::{Lua, Result};

#[test]
fn test_byte_string_round_trip() -> Result<()> {
    let lua = Lua::new();

    lua.load(
        r#"
        invalid_sequence_identifier = "\160\161"
        invalid_2_octet_sequence_2nd = "\195\040"
        invalid_3_octet_sequence_2nd = "\226\040\161"
        invalid_3_octet_sequence_3rd = "\226\130\040"
        invalid_4_octet_sequence_2nd = "\240\040\140\188"
        invalid_4_octet_sequence_3rd = "\240\144\040\188"
        invalid_4_octet_sequence_4th = "\240\040\140\040"

        an_actual_string = "Hello, world!"
    "#,
    )
    .exec()?;

    let globals = lua.globals();

    let isi = globals.get::<BString>("invalid_sequence_identifier")?;
    assert_eq!(isi, [0xa0, 0xa1].as_ref());

    let i2os2 = globals.get::<BString>("invalid_2_octet_sequence_2nd")?;
    assert_eq!(i2os2, [0xc3, 0x28].as_ref());

    let i3os2 = globals.get::<BString>("invalid_3_octet_sequence_2nd")?;
    assert_eq!(i3os2, [0xe2, 0x28, 0xa1].as_ref());

    let i3os3 = globals.get::<BString>("invalid_3_octet_sequence_3rd")?;
    assert_eq!(i3os3, [0xe2, 0x82, 0x28].as_ref());

    let i4os2 = globals.get::<BString>("invalid_4_octet_sequence_2nd")?;
    assert_eq!(i4os2, [0xf0, 0x28, 0x8c, 0xbc].as_ref());

    let i4os3 = globals.get::<BString>("invalid_4_octet_sequence_3rd")?;
    assert_eq!(i4os3, [0xf0, 0x90, 0x28, 0xbc].as_ref());

    let i4os4 = globals.get::<BString>("invalid_4_octet_sequence_4th")?;
    assert_eq!(i4os4, [0xf0, 0x28, 0x8c, 0x28].as_ref());

    let aas = globals.get::<BString>("an_actual_string")?;
    assert_eq!(aas, b"Hello, world!".as_ref());

    globals.set("bstr_invalid_sequence_identifier", isi.as_ref() as &BStr)?;
    globals.set("bstr_invalid_2_octet_sequence_2nd", i2os2.as_ref() as &BStr)?;
    globals.set("bstr_invalid_3_octet_sequence_2nd", i3os2.as_ref() as &BStr)?;
    globals.set("bstr_invalid_3_octet_sequence_3rd", i3os3.as_ref() as &BStr)?;
    globals.set("bstr_invalid_4_octet_sequence_2nd", i4os2.as_ref() as &BStr)?;
    globals.set("bstr_invalid_4_octet_sequence_3rd", i4os3.as_ref() as &BStr)?;
    globals.set("bstr_invalid_4_octet_sequence_4th", i4os4.as_ref() as &BStr)?;
    globals.set("bstr_an_actual_string", aas.as_ref() as &BStr)?;

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

    globals.set("bstring_invalid_sequence_identifier", isi)?;
    globals.set("bstring_invalid_2_octet_sequence_2nd", i2os2)?;
    globals.set("bstring_invalid_3_octet_sequence_2nd", i3os2)?;
    globals.set("bstring_invalid_3_octet_sequence_3rd", i3os3)?;
    globals.set("bstring_invalid_4_octet_sequence_2nd", i4os2)?;
    globals.set("bstring_invalid_4_octet_sequence_3rd", i4os3)?;
    globals.set("bstring_invalid_4_octet_sequence_4th", i4os4)?;
    globals.set("bstring_an_actual_string", aas)?;

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
