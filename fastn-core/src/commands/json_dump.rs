pub async fn json_dump(
    config: &fastn_core::Config,
    stage: &str,
    path: Option<&str>,
    with_null: bool,
) -> fastn_core::Result<()> {
    let mut documents = std::collections::BTreeMap::from_iter(
        config
            .get_files(&config.package)
            .await?
            .into_iter()
            .map(|v| (v.get_id(), v)),
    );

    if let Some(path) = path {
        let file = documents.values().find(|v| v.get_id().eq(path)).ok_or(
            fastn_core::Error::UsageError {
                message: format!("{} not found in the package", path),
            },
        )?;

        let value = get_ftd_json(file, stage)?;
        println!(
            "{}",
            if with_null {
                fastn_core::utils::value_to_colored_string(&value, 1)
            } else {
                fastn_core::utils::value_to_colored_string_without_null(&value, 1)
            }
        );

        return Ok(());
    }
    unimplemented!()
}

fn get_ftd_json(file: &fastn_core::File, stage: &str) -> fastn_core::Result<serde_json::Value> {
    let document = if let fastn_core::File::Ftd(document) = file {
        document
    } else {
        return Err(fastn_core::Error::UsageError {
            message: format!("{} is not an ftd file", file.get_id()),
        });
    };

    match stage {
        "p1" => get_p1_json(document),
        _ => unimplemented!(),
    }
}

fn get_p1_json(document: &fastn_core::Document) -> fastn_core::Result<serde_json::Value> {
    let p1 = ftd::p1::parse(
        document.content.as_str(),
        document.id_with_package().as_str(),
    )?;
    let value = serde_json::to_value(p1)?;

    Ok(value)
}