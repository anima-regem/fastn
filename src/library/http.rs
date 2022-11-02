pub async fn processor<'a>(
    section: &ftd::p1::Section,
    doc: &ftd::p2::TDoc<'a>,
    config: &fpm::Config,
) -> ftd::p1::Result<ftd::Value> {
    {
        let method = section
            .header
            .str_with_default(doc.name, section.line_number, "method", "GET")?
            .to_lowercase();

        if method != "get" {
            return ftd::p2::utils::e2(
                format!("only GET method is allowed, found: {}", method),
                doc.name,
                section.line_number,
            );
        }
    }

    let url = match section
        .header
        .string_optional(doc.name, section.line_number, "url")?
    {
        Some(v) => v,
        None => {
            return ftd::p2::utils::e2(
                "'url' key is required when using `$processor$: http`",
                doc.name,
                section.line_number,
            )
        }
    };

    let mut url =
        utils::get_clean_url(config, url.as_str()).map_err(|e| ftd::p1::Error::ParseError {
            message: format!("invalid url: {:?}", e),
            doc_id: doc.name.to_string(),
            line_number: section.line_number,
        })?;

    for (line, key, value) in section.header.0.iter() {
        if key == "$processor$" || key == "url" || key == "method" {
            continue;
        }

        // 1 id: $query.id
        // After resolve headers: id:1234(value of $query.id)
        if value.starts_with('$') {
            if let Some(value) = doc.get_value(*line, value)?.to_string() {
                url.query_pairs_mut().append_pair(key, &value);
            }
        } else {
            url.query_pairs_mut().append_pair(key, value);
        }
    }

    println!("calling `http` processor with url: {}", &url);

    let response = match crate::http::http_get_with_cookie(
        url.as_str(),
        config.request.as_ref().and_then(|v| v.cookies_string()),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            return ftd::p2::utils::e2(
                format!("HTTP::get failed: {:?}", e),
                doc.name,
                section.line_number,
            )
        }
    };

    let response_string = String::from_utf8(response).map_err(|e| ftd::p1::Error::ParseError {
        message: format!("`http` processor API response error: {}", e),
        doc_id: doc.name.to_string(),
        line_number: section.line_number,
    })?;
    let response_json: serde_json::Value =
        serde_json::from_str(&response_string).map_err(|e| ftd::p1::Error::Serde { source: e })?;

    doc.from_json(&response_json, section)
}

// Need to pass the request object also
// From request get the url, get query parameters, get the data from body(form data, post data)
pub fn request_data_processor<'a>(
    section: &ftd::p1::Section,
    doc: &ftd::p2::TDoc<'a>,
    config: &fpm::Config,
) -> ftd::p1::Result<ftd::Value> {
    // TODO: URL params not yet handled
    let req = match config.request.as_ref() {
        Some(v) => v,
        None => {
            return ftd::p2::utils::e2(
                "HttpRequest object should not be null",
                doc.name,
                section.line_number,
            )
        }
    };
    let mut data = req.query().clone();

    let mut path_parameters = std::collections::HashMap::new();
    for (name, value) in config.path_parameters.iter() {
        let json_value = value.to_serde_value().ok_or(ftd::p1::Error::ParseError {
            message: format!("ftd value cannot be parsed to json: name: {}", name),
            doc_id: doc.name.to_string(),
            line_number: section.line_number,
        })?;
        path_parameters.insert(name.to_string(), json_value);
    }

    data.extend(path_parameters);

    match req.body_as_json() {
        Ok(Some(b)) => {
            data.extend(b);
        }
        Ok(None) => {}
        Err(e) => {
            return ftd::p2::utils::e2(
                format!("Error while parsing request body: {:?}", e),
                doc.name,
                section.line_number,
            )
        }
    }

    doc.from_json(&data, section)
}

mod utils {
    // this url can be start with http of /-/package-name/
    // It will return url with end-point, if package or dependency contains endpoint in them
    pub fn get_clean_url(config: &fpm::Config, url: &str) -> fpm::Result<url::Url> {
        if url.starts_with("http") {
            return Ok(url::Url::parse(url)?);
        }

        // if path starts with /-/package-name, so it trim the package and return the remaining url
        fn path_start_with(path: &str, package_name: &str) -> Option<String> {
            let package_name = format!("/-/{}", package_name.trim().trim_matches('/'));
            if path.starts_with(package_name.as_str()) {
                return Some(path.trim_start_matches(package_name.as_str()).to_string());
            }
            None
        }

        // This is for current package
        if let Some(remaining_url) = path_start_with(url, config.package.name.as_str()) {
            let end_point = match config.package.endpoint.as_ref() {
                Some(ep) => ep,
                None => {
                    return Err(fpm::Error::GenericError(format!(
                        "package does not contain the endpoint: {:?}",
                        config.package.name
                    )));
                }
            };
            return Ok(url::Url::parse(
                format!("{}{}", end_point, remaining_url).as_str(),
            )?);
        }

        // This is for dependency packages
        let deps_ep = config.package.dep_with_ep();
        for (dep, ep) in deps_ep {
            if let Some(remaining_url) = path_start_with(url, dep.name.as_str()) {
                return Ok(url::Url::parse(
                    format!("{}{}", ep, remaining_url).as_str(),
                )?);
            }
        }

        Err(fpm::Error::GenericError(format!(
            "http-processor: end-point not found url: {}",
            url
        )))
    }
}
