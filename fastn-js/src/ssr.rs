pub fn ssr(js: &str) -> String {
    let fastn_js = include_str!("../fastn.js");
    let dom_js = include_str!("../dom.js");
    let utils_js = include_str!("../utils.js");
    let virtual_js = include_str!("../virtual.js");
    let js = format!("{fastn_js}{dom_js}{utils_js}{virtual_js}{js}");
    std::fs::write("test.js", &js).unwrap();
    rquickjs::Context::full(&rquickjs::Runtime::new().unwrap())
        .unwrap()
        .with(|ctx| ctx.eval::<String, _>(js).unwrap())
}
