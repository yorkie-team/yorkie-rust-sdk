use yorkie_core::{Document, JsonArray, JsonObject, Result};

#[test]
fn updates_a_local_document_root() -> Result<()> {
    let mut doc = Document::new("test-doc");

    doc.update(|root| {
        root.set("title", "hello")?;
        root.set("done", false)?;
        root.set("count", 1i64)?;

        let mut profile = JsonObject::new();
        profile.set("name", "yorkie")?;
        root.set("profile", profile)?;

        root.set("todos", JsonArray::new())?;
        root.get_array_mut("todos")?
            .push("write tests")
            .push("sync");

        Ok(())
    })?;

    assert_eq!(
        r#"{"count":1,"done":false,"profile":{"name":"yorkie"},"title":"hello","todos":["write tests","sync"]}"#,
        doc.to_sorted_json()
    );

    Ok(())
}

#[test]
fn preserves_document_key_as_given() {
    let doc = Document::new("invalid key");

    assert_eq!("invalid key", doc.key());
}

#[test]
fn removes_object_member() -> Result<()> {
    let mut doc = Document::new("test-doc");

    doc.update(|root| {
        let mut profile = JsonObject::new();
        profile.set("name", "yorkie")?;
        profile.set("active", true)?;
        root.set("profile", profile)?;

        let profile = root.get_object_mut("profile")?;
        assert!(profile.remove("name").is_some());
        assert!(profile.remove("missing").is_none());

        Ok(())
    })?;

    assert_eq!(r#"{"profile":{"active":true}}"#, doc.to_sorted_json());

    Ok(())
}
