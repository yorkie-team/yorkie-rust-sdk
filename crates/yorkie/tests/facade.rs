use yorkie::{Document, Result};

#[test]
fn facade_exports_document_api() -> Result<()> {
    let mut doc = Document::new("test-doc");

    doc.update(|root| {
        root.set("title", "hello");
        Ok(())
    })?;

    assert_eq!(r#"{"title":"hello"}"#, doc.to_sorted_json());

    Ok(())
}
