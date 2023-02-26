"use strict";
window.ftd = (function () {
    let ftd_data = {};
    let exports = {};
    exports.init = function (id, data) {
        let element = document.getElementById(data);
        if (!!element) {
            ftd_data[id] = JSON.parse(element.innerText);
            window.ftd.post_init();
        }
    };
    exports.data = ftd_data;
    function handle_function(evt, id, action, obj, function_arguments) {
        console.log(id, action);
        console.log(action.name);
        let argument;
        for (argument in action.values) {
            if (action.values.hasOwnProperty(argument)) {
                // @ts-ignore
                let value = action.values[argument][1] !== undefined ? action.values[argument][1] : action.values[argument];
                if (typeof value === 'object') {
                    let function_argument = value;
                    if (!!function_argument && !!function_argument.reference) {
                        let obj_value = null;
                        let obj_checked = null;
                        try {
                            obj_value = obj.value;
                            obj_checked = obj.checked;
                        }
                        catch (_a) {
                            obj_value = null;
                            obj_checked = null;
                        }
                        let value = resolve_reference(function_argument.reference, ftd_data[id], obj_value, obj_checked);
                        if (!!function_argument.mutable) {
                            function_argument.value = value;
                            function_arguments.push(function_argument);
                        }
                        else {
                            function_arguments.push(deepCopy(value));
                        }
                    }
                }
                else {
                    function_arguments.push(value);
                }
            }
        }
        return window[action.name](...function_arguments, function_arguments, ftd_data[id], id);
    }
    function handle_event(evt, id, action, obj) {
        let function_arguments = [];
        handle_function(evt, id, action, obj, function_arguments);
        // @ts-ignore
        if (function_arguments["CHANGE_VALUE"] !== false) {
            change_value(function_arguments, ftd_data[id], id);
        }
    }
    exports.handle_event = function (evt, id, event, obj) {
        console_log(id, event);
        let actions = JSON.parse(event);
        for (const action in actions) {
            handle_event(evt, id, actions[action], obj);
        }
    };
    exports.handle_function = function (evt, id, event, obj) {
        console_log(id, event);
        let actions = JSON.parse(event);
        let function_arguments = [];
        return handle_function(evt, id, actions, obj, function_arguments);
    };
    exports.get_value = function (id, variable) {
        let data = ftd_data[id];
        let [var_name, _] = get_name_and_remaining(variable);
        if (data[var_name] === undefined && data[variable] === undefined) {
            console_log(variable, "is not in data, ignoring");
            return;
        }
        return get_data_value(data, variable);
    };
    exports.set_string_for_all = function (variable, value) {
        for (let id in ftd_data) {
            if (!ftd_data.hasOwnProperty(id)) {
                continue;
            }
            // @ts-ignore
            exports.set_value_by_id(id, variable, value);
        }
    };
    exports.set_bool_for_all = function (variable, value) {
        for (let id in ftd_data) {
            if (!ftd_data.hasOwnProperty(id)) {
                continue;
            }
            // @ts-ignore
            exports.set_bool(id, variable, value);
        }
    };
    exports.set_bool = function (id, variable, value) {
        window.ftd.set_value_by_id(id, variable, value);
    };
    exports.set_value = function (variable, value) {
        window.ftd.set_value_by_id("main", variable, value);
    };
    exports.set_value_by_id = function (id, variable, value) {
        let data = ftd_data[id];
        let [var_name, remaining] = data[variable] === undefined
            ? get_name_and_remaining(variable)
            : [variable, null];
        if (data[var_name] === undefined && data[variable] === undefined) {
            console_log(variable, "is not in data, ignoring");
            return;
        }
        window.ftd.delete_list(var_name, id);
        if (!!window["set_value_" + id] && !!window["set_value_" + id][var_name]) {
            window["set_value_" + id][var_name](data, value, remaining);
        }
        else {
            set_data_value(data, variable, value);
        }
        window.ftd.create_list(var_name, id);
    };
    exports.is_empty = function (str) {
        return (!str || str.length === 0);
    };
    exports.set_list = function (array, value, args, data, id) {
        args["CHANGE_VALUE"] = false;
        window.ftd.clear(array, args, data, id);
        args[0].value = value;
        change_value(args, data, id);
        window.ftd.create_list(args[0].reference, id);
        return array;
    };
    exports.create_list = function (array_name, id) {
        if (!!window.dummy_data_main && !!window.dummy_data_main[array_name]) {
            let data = ftd_data[id];
            let dummys = window.dummy_data_main[array_name](data);
            for (let i in dummys) {
                let [htmls, data_id, start_index] = dummys[i];
                for (let i in htmls) {
                    let nodes = stringToHTML(htmls[i]);
                    let main = document.querySelector(`[data-id="${data_id}"]`);
                    main === null || main === void 0 ? void 0 : main.insertBefore(nodes.children[0], main.children[start_index + parseInt(i)]);
                    /*for (var j = 0, len = nodes.childElementCount; j < len; ++j) {
                        main?.insertBefore(nodes.children[j], main.children[start_index + parseInt(i)]);
                    }*/
                }
            }
        }
    };
    exports.append = function (array, value, args, data, id) {
        array.push(value);
        args["CHANGE_VALUE"] = false;
        args[0].value = array;
        change_value(args, data, id);
        if (!!window.dummy_data_main && !!window.dummy_data_main[args[0].reference]) {
            // @ts-ignore
            let list = resolve_reference(args[0].reference, data);
            let dummys = window.dummy_data_main[args[0].reference](data, "LAST");
            for (let i in dummys) {
                let [html, data_id, start_index] = dummys[i];
                let nodes = stringToHTML(html);
                let main = document.querySelector(`[data-id="${data_id}"]`);
                for (var j = 0, len = nodes.childElementCount; j < len; ++j) {
                    // @ts-ignore
                    main.insertBefore(nodes.children[j], main.children[start_index + list.length - 1]);
                }
            }
        }
        return array;
    };
    exports.insert_at = function (array, value, idx, args, data, id) {
        array.push(value);
        args["CHANGE_VALUE"] = false;
        args[0].value = array;
        change_value(args, data, id);
        if (!!window.dummy_data_main && !!window.dummy_data_main[args[0].reference]) {
            // @ts-ignore
            let list = resolve_reference(args[0].reference, data);
            let dummys = window.dummy_data_main[args[0].reference](data, "LAST");
            for (let i in dummys) {
                let [html, data_id, start_index] = dummys[i];
                let nodes = stringToHTML(html);
                let main = document.querySelector(`[data-id="${data_id}"]`);
                if (idx >= list.length) {
                    idx = list.length - 1;
                }
                else if (idx < 0) {
                    idx = 0;
                }
                // @ts-ignore
                main.insertBefore(nodes.children[0], main.children[start_index + idx]);
            }
        }
        return array;
    };
    exports.clear = function (array, args, data, id) {
        args["CHANGE_VALUE"] = false;
        // @ts-ignore
        window.ftd.delete_list(args[0].reference, id);
        args[0].value = [];
        change_value(args, data, id);
        return array;
    };
    exports.delete_list = function (array_name, id) {
        if (!!window.dummy_data_main && !!window.dummy_data_main[array_name]) {
            let data = ftd_data[id];
            let length = resolve_reference(array_name, data, null, null).length;
            let dummys = window.dummy_data_main[array_name](data);
            for (let j in dummys) {
                let [_, data_id, start_index] = dummys[j];
                let main = document.querySelector(`[data-id="${data_id}"]`);
                for (var i = length - 1 + start_index; i >= start_index; i--) {
                    main === null || main === void 0 ? void 0 : main.removeChild(main.children[i]);
                }
            }
        }
    };
    exports.delete_at = function (array, idx, args, data, id) {
        // @ts-ignore
        let length = resolve_reference(args[0].reference, data).length;
        if (idx >= length) {
            idx = length - 1;
        }
        else if (idx < 0) {
            idx = 0;
        }
        array.splice(idx, 1);
        args["CHANGE_VALUE"] = false;
        args[0].value = array;
        change_value(args, data, id);
        if (!!window.dummy_data_main && !!window.dummy_data_main[args[0].reference]) {
            let dummys = window.dummy_data_main[args[0].reference](data);
            for (let i in dummys) {
                let [_, data_id, start_index] = dummys[i];
                let main = document.querySelector(`[data-id="${data_id}"]`);
                main === null || main === void 0 ? void 0 : main.removeChild(main.children[start_index + idx]);
            }
        }
        return array;
    };
    exports.http = function (url, method, ...request_data) {
        let method_name = method.trim().toUpperCase();
        if (method_name == "GET") {
            let query_parameters = new URLSearchParams();
            // @ts-ignore
            for (let [header, value] of Object.entries(request_data)) {
                if (header != "url" && header != "function" && header != "method") {
                    let [key, val] = value.length == 2 ? value : [header, value];
                    query_parameters.set(key, val);
                }
            }
            let query_string = query_parameters.toString();
            if (query_string) {
                let get_url = url + "?" + query_parameters.toString();
                window.location.href = get_url;
            }
            else {
                window.location.href = url;
            }
            return;
        }
        let json = request_data[0];
        if (request_data.length !== 1 || (request_data[0].length === 2 && Array.isArray(request_data[0]))) {
            let new_json = {};
            // @ts-ignore
            for (let [header, value] of Object.entries(request_data)) {
                let [key, val] = value.length == 2 ? value : [header, value];
                new_json[key] = val;
            }
            json = new_json;
        }
        let xhr = new XMLHttpRequest();
        xhr.open(method_name, url);
        xhr.setRequestHeader("Accept", "application/json");
        xhr.setRequestHeader("Content-Type", "application/json");
        xhr.onreadystatechange = function () {
            if (xhr.readyState !== 4) {
                // this means request is still underway
                // https://developer.mozilla.org/en-US/docs/Web/API/XMLHttpRequest/readyState
                return;
            }
            if (xhr.status > 500) {
                console.log("Error in calling url: ", request_data.url, xhr.responseText);
                return;
            }
            let response = JSON.parse(xhr.response);
            if (!!response && !!response.redirect) {
                // Warning: we don't handle header location redirect
                window.location.href = response.redirect;
            }
            else if (!!response && !!response.reload) {
                window.location.reload();
            }
            else {
                let data = {};
                if (!!response.errors) {
                    for (let key of Object.keys(response.errors)) {
                        let value = response.errors[key];
                        if (Array.isArray(value)) {
                            // django returns a list of strings
                            value = value.join(" ");
                            // also django does not append `-error`
                            key = key + "-error";
                        }
                        // @ts-ignore
                        data[key] = value;
                    }
                }
                if (!!response.data) {
                    if (!!data) {
                        console_log("both .errrors and .data are present in response, ignoring .data");
                    }
                    else {
                        data = response.data;
                    }
                }
                for (let ftd_variable of Object.keys(data)) {
                    // @ts-ignore
                    window.ftd.set_value(ftd_variable, data[ftd_variable]);
                }
            }
        };
        xhr.send(JSON.stringify(json));
    };
    // source: https://stackoverflow.com/questions/400212/ (cc-by-sa)
    exports.copy_to_clipboard = function (text) {
        if (!navigator.clipboard) {
            fallbackCopyTextToClipboard(text);
            return;
        }
        navigator.clipboard.writeText(text).then(function () {
            console.log('Async: Copying to clipboard was successful!');
        }, function (err) {
            console.error('Async: Could not copy text: ', err);
        });
    };
    exports.component_data = function (component) {
        let data = {};
        for (let idx in component.getAttributeNames()) {
            let argument = component.getAttributeNames()[idx];
            // @ts-ignore
            data[argument] = eval(component.getAttribute(argument));
        }
        return data;
    };
    return exports;
})();
window.ftd.post_init = function () {
    const DARK_MODE = "ftd#dark-mode";
    const SYSTEM_DARK_MODE = "ftd#system-dark-mode";
    const FOLLOW_SYSTEM_DARK_MODE = "ftd#follow-system-dark-mode";
    const DARK_MODE_COOKIE = "ftd-dark-mode";
    const COOKIE_SYSTEM_LIGHT = "system-light";
    const COOKIE_SYSTEM_DARK = "system-dark";
    const COOKIE_DARK_MODE = "dark";
    const COOKIE_LIGHT_MODE = "light";
    const DARK_MODE_CLASS = "fpm-dark";
    const MOBILE_CLASS = "ftd-mobile";
    const XL_CLASS = "ftd-xl";
    const FTD_DEVICE = "ftd#device";
    const FTD_BREAKPOINT_WIDTH = "ftd#breakpoint-width";
    const FTD_THEME_COLOR = "ftd#theme-color";
    const THEME_COLOR_META = "theme-color";
    const MARKDOWN_COLOR = "ftd#markdown-color";
    const MARKDOWN_BACKGROUND_COLOR = "ftd#markdown-background-color";
    let last_device;
    function initialise_device() {
        last_device = get_device();
        console_log("last_device", last_device);
        window.ftd.set_string_for_all(FTD_DEVICE, last_device);
    }
    window.onresize = function () {
        let current = get_device();
        if (current === last_device) {
            return;
        }
        window.ftd.set_string_for_all(FTD_DEVICE, current);
        last_device = current;
        console_log("last_device", last_device);
    };
    /*function update_markdown_colors() {
       // remove all colors from ftd.css: copy every deleted stuff in this function
       let markdown_style_sheet = document.createElement('style');


       markdown_style_sheet.innerHTML = `
       .ft_md a {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".link.light")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".link.light")};
       }
       body.fpm-dark .ft_md a {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".link.dark")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".link.dark")};
       }

       .ft_md code {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".code.light")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".code.light")};
       }
       body.fpm-dark .ft_md code {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".code.dark")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".code.dark")};
       }

       .ft_md a:visited {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".link-visited.light")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".link-visited.light")};
       }
       body.fpm-dark .ft_md a:visited {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".link-visited.dark")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".link-visited.dark")};
       }

       .ft_md a code {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".link-code.light")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".link-code.light")};
       }
       body.fpm-dark .ft_md a code {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".link-code.dark")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".link-code.dark")};
       }

       .ft_md a:visited code {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".link-visited-code.light")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".link-visited-code.light")};
       }
       body.fpm-dark .ft_md a:visited code {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".link-visited-code.dark")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".link-visited-code.dark")};
       }

       .ft_md ul ol li:before {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".ul-ol-li-before.light")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".ul-ol-li-before.light")};
       }
       body.fpm-dark .ft_md ul ol li:before {
           color: ${window.ftd.get_value("main", MARKDOWN_COLOR + ".ul-ol-li-before.dark")};
           background-color: ${window.ftd.get_value("main", MARKDOWN_BACKGROUND_COLOR + ".ul-ol-li-before.dark")};
       }
       `;

       document.getElementsByTagName('head')[0].appendChild(markdown_style_sheet);
   }*/
    function get_device() {
        // not at all sure about this functions logic.
        let width = window.innerWidth;
        // in future we may want to have more than one break points, and then
        // we may also want the theme builders to decide where the breakpoints
        // should go. we should be able to fetch fpm variables here, or maybe
        // simply pass the width, user agent etc to fpm and let people put the
        // checks on width user agent etc, but it would be good if we can
        // standardize few breakpoints. or maybe we should do both, some
        // standard breakpoints and pass the raw data.
        // we would then rename this function to detect_device() which will
        // return one of "desktop", "tablet", "mobile". and also maybe have
        // another function detect_orientation(), "landscape" and "portrait" etc,
        // and instead of setting `fpm#mobile: boolean` we set `fpm-ui#device`
        // and `fpm#view-port-orientation` etc.
        let mobile_breakpoint = window.ftd.get_value("main", FTD_BREAKPOINT_WIDTH + ".mobile");
        if (width <= mobile_breakpoint) {
            document.body.classList.add(MOBILE_CLASS);
            if (document.body.classList.contains(XL_CLASS)) {
                document.body.classList.remove(XL_CLASS);
            }
            return "mobile";
        }
        /*if (width > desktop_breakpoint) {
            document.body.classList.add(XL_CLASS);
            if (document.body.classList.contains(MOBILE_CLASS)) {
                document.body.classList.remove(MOBILE_CLASS);
            }
            return "xl";
        }*/
        if (document.body.classList.contains(MOBILE_CLASS)) {
            document.body.classList.remove(MOBILE_CLASS);
        }
        /*if (document.body.classList.contains(XL_CLASS)) {
            document.body.classList.remove(XL_CLASS);
        }*/
        return "desktop";
    }
    /*
        ftd.dark-mode behaviour:

        ftd.dark-mode is a boolean, default false, it tells the UI to show
        the UI in dark or light mode. Themes should use this variable to decide
        which mode to show in UI.

        ftd.follow-system-dark-mode, boolean, default true, keeps track if
        we are reading the value of `dark-mode` from system preference, or user
        has overridden the system preference.

        These two variables must not be set by ftd code directly, but they must
        use `$on-click$: message-host enable-dark-mode`, to ignore system
        preference and use dark mode. `$on-click$: message-host
        disable-dark-mode` to ignore system preference and use light mode and
        `$on-click$: message-host follow-system-dark-mode` to ignore user
        preference and start following system preference.

        we use a cookie: `ftd-dark-mode` to store the preference. The cookie can
        have three values:

           cookie missing /          user wants us to honour system preference
               system-light          and currently its light.

           system-dark               follow system and currently its dark.

           light:                    user prefers light

           dark:                     user prefers light

        We use cookie instead of localstorage so in future `fpm-repo` can see
        users preferences up front and renders the HTML on service wide
        following user's preference.

     */
    window.enable_dark_mode = function () {
        // TODO: coalesce the two set_bool-s into one so there is only one DOM
        //       update
        window.ftd.set_bool_for_all(DARK_MODE, true);
        window.ftd.set_bool_for_all(FOLLOW_SYSTEM_DARK_MODE, false);
        window.ftd.set_bool_for_all(SYSTEM_DARK_MODE, system_dark_mode());
        document.body.classList.add(DARK_MODE_CLASS);
        set_cookie(DARK_MODE_COOKIE, COOKIE_DARK_MODE);
        update_theme_color();
    };
    window.enable_light_mode = function () {
        // TODO: coalesce the two set_bool-s into one so there is only one DOM
        //       update
        window.ftd.set_bool_for_all(DARK_MODE, false);
        window.ftd.set_bool_for_all(FOLLOW_SYSTEM_DARK_MODE, false);
        window.ftd.set_bool_for_all(SYSTEM_DARK_MODE, system_dark_mode());
        if (document.body.classList.contains(DARK_MODE_CLASS)) {
            document.body.classList.remove(DARK_MODE_CLASS);
        }
        set_cookie(DARK_MODE_COOKIE, COOKIE_LIGHT_MODE);
        update_theme_color();
    };
    window.enable_system_mode = function () {
        // TODO: coalesce the two set_bool-s into one so there is only one DOM
        //       update
        window.ftd.set_bool_for_all(FOLLOW_SYSTEM_DARK_MODE, true);
        window.ftd.set_bool_for_all(SYSTEM_DARK_MODE, system_dark_mode());
        if (system_dark_mode()) {
            window.ftd.set_bool_for_all(DARK_MODE, true);
            document.body.classList.add(DARK_MODE_CLASS);
            set_cookie(DARK_MODE_COOKIE, COOKIE_SYSTEM_DARK);
        }
        else {
            window.ftd.set_bool_for_all(DARK_MODE, false);
            if (document.body.classList.contains(DARK_MODE_CLASS)) {
                document.body.classList.remove(DARK_MODE_CLASS);
            }
            set_cookie(DARK_MODE_COOKIE, COOKIE_SYSTEM_LIGHT);
        }
        update_theme_color();
    };
    function update_theme_color() {
        let theme_color = window.ftd.get_value("main", FTD_THEME_COLOR);
        if (!!theme_color) {
            document.body.style.backgroundColor = FTD_THEME_COLOR;
            set_meta(THEME_COLOR_META, theme_color);
        }
        else {
            document.body.style.backgroundColor = FTD_THEME_COLOR;
            delete_meta(THEME_COLOR_META);
        }
    }
    function set_meta(name, value) {
        let meta = document.querySelector("meta[name=" + name + "]");
        if (!!meta) {
            meta.content = value;
        }
        else {
            meta = document.createElement('meta');
            meta.name = name;
            meta.content = value;
            document.getElementsByTagName('head')[0].appendChild(meta);
        }
    }
    function delete_meta(name) {
        let meta = document.querySelector("meta[name=" + name + "]");
        if (!!meta) {
            meta.remove();
        }
    }
    function set_cookie(name, value) {
        document.cookie = name + "=" + value + "; path=/";
    }
    function system_dark_mode() {
        return !!(window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches);
    }
    function initialise_dark_mode() {
        update_dark_mode();
        start_watching_dark_mode_system_preference();
    }
    function get_cookie(name, def) {
        // source: https://stackoverflow.com/questions/5639346/
        let regex = document.cookie.match('(^|;)\\s*' + name + '\\s*=\\s*([^;]+)');
        return regex !== null ? regex.pop() : def;
    }
    function update_dark_mode() {
        let current_dark_mode_cookie = get_cookie(DARK_MODE_COOKIE, COOKIE_SYSTEM_LIGHT);
        switch (current_dark_mode_cookie) {
            case COOKIE_SYSTEM_LIGHT:
            case COOKIE_SYSTEM_DARK:
                window.enable_system_mode();
                break;
            case COOKIE_LIGHT_MODE:
                window.enable_light_mode();
                break;
            case COOKIE_DARK_MODE:
                window.enable_dark_mode();
                break;
            default:
                console_log("cookie value is wrong", current_dark_mode_cookie);
                window.enable_system_mode();
        }
    }
    function start_watching_dark_mode_system_preference() {
        window.matchMedia('(prefers-color-scheme: dark)').addEventListener("change", update_dark_mode);
    }
    initialise_dark_mode();
    initialise_device();
    // update_markdown_colors();
};
function console_log(...message) {
    if (true) { // false
        console.log(...message);
    }
}
function isObject(obj) {
    return obj != null && typeof obj === 'object' && obj === Object(obj);
}
function stringToHTML(str) {
    var parser = new DOMParser();
    var doc = parser.parseFromString(str, 'text/html');
    return doc.body;
}
;
function get_name_and_remaining(name) {
    let part1 = "";
    let pattern_to_split_at = name;
    let parent_split = split_once(name, "#");
    if (parent_split.length === 2) {
        part1 = parent_split[0] + "#";
        pattern_to_split_at = parent_split[1];
    }
    parent_split = split_once(pattern_to_split_at, ".");
    if (parent_split.length === 2) {
        return [part1 + parent_split[0], parent_split[1]];
    }
    return [name, null];
}
function split_once(name, split_at) {
    const i = name.indexOf(split_at);
    if (i === -1) {
        return [name];
    }
    return [name.slice(0, i), name.slice(i + 1)];
}
function deepCopy(object) {
    if (isObject(object)) {
        return JSON.parse(JSON.stringify(object));
    }
    return object;
}
function change_value(function_arguments, data, id) {
    for (const a in function_arguments) {
        if (isFunctionArgument(function_arguments[a])) {
            if (!!function_arguments[a]["reference"]) {
                let reference = function_arguments[a]["reference"];
                let [var_name, remaining] = (!!data[reference]) ? [reference, null] : get_name_and_remaining(reference);
                if (var_name === "ftd#dark-mode") {
                    if (!!function_arguments[a]["value"]) {
                        window.enable_dark_mode();
                    }
                    else {
                        window.enable_light_mode();
                    }
                }
                else if (!!window["set_value_" + id] && !!window["set_value_" + id][var_name]) {
                    window["set_value_" + id][var_name](data, function_arguments[a]["value"], remaining);
                }
                else {
                    set_data_value(data, reference, function_arguments[a]["value"]);
                }
            }
        }
    }
}
function isFunctionArgument(object) {
    return object.value !== undefined;
}
String.prototype.format = function () {
    var formatted = this;
    for (var i = 0; i < arguments.length; i++) {
        var regexp = new RegExp('\\{' + i + '\\}', 'gi');
        formatted = formatted.replace(regexp, arguments[i]);
    }
    return formatted;
};
String.prototype.replace_format = function () {
    var formatted = this;
    if (arguments.length > 0) {
        // @ts-ignore
        for (let [header, value] of Object.entries(arguments[0])) {
            var regexp = new RegExp('\\{(' + header + '(\\..*?)?)\\}', 'gi');
            let matching = formatted.match(regexp);
            for (let i in matching) {
                try {
                    // @ts-ignore
                    formatted = formatted.replace(matching[i], resolve_reference(matching[i].substring(1, matching[i].length - 1), arguments[0]));
                }
                catch (e) {
                    continue;
                }
            }
        }
    }
    return formatted;
};
function set_data_value(data, name, value) {
    if (!!data[name]) {
        data[name] = deepCopy(set(data[name], null, value));
        return;
    }
    let [var_name, remaining] = get_name_and_remaining(name);
    let initial_value = data[var_name];
    data[var_name] = deepCopy(set(initial_value, remaining, value));
    // tslint:disable-next-line:no-shadowed-variable
    function set(initial_value, remaining, value) {
        if (!remaining) {
            return value;
        }
        let [p1, p2] = split_once(remaining, ".");
        initial_value[p1] = set(initial_value[p1], p2, value);
        return initial_value;
    }
}
function resolve_reference(reference, data, value, checked) {
    if (reference === "VALUE") {
        return value;
    }
    if (reference === "CHECKED") {
        return checked;
    }
    if (!!data[reference]) {
        return deepCopy(data[reference]);
    }
    let [var_name, remaining] = get_name_and_remaining(reference);
    let initial_value = data[var_name];
    while (!!remaining) {
        let [p1, p2] = split_once(remaining, ".");
        initial_value = initial_value[p1];
        remaining = p2;
    }
    return deepCopy(initial_value);
}
function get_data_value(data, name) {
    return resolve_reference(name, data, null, null);
}
function JSONstringify(f) {
    if (typeof f === 'object') {
        return JSON.stringify(f);
    }
    else {
        return f;
    }
}
function get_color_value(cs, category, color_name) {
    let category_data = cs[category];
    let color_data = category_data[color_name];
    let color_value = color_data['value'];
    return color_value;
}
function styled_body(body) {
    return `
<span style="color:#c0c5ce;">${body}</span>`;
}
function styled_section(line) {
    var section_type_title = line.replace("-- ", "").replace(":", "");
    var result = `
<span style="color:#65737e;">-- </span><span style="color:#ebcb8b;">${section_type_title}</span><span style="color:#65737e;">: </span>`;
    return result;
}
function styled_header(line) {
    var header_splits = line.split(":");
    var result = `
<span style="color:#b48ead;">${header_splits[0]}</span><span style="color:#65737e;">: </span><span style="color:#d08770;">${header_splits[1].trim()} </span>`;
    return result;
}
function apply_style(s) {
    var result = new String();
    const lines = s.split(/\r\n|\r|\n/);
    for (var line of lines) {
        if (line.trim().length == 0) {
            // Empty line
            result = result.concat(styled_body(" "));
        }
        else if (line.startsWith("--")) {
            // Section top
            result = result.concat(styled_section(line));
        }
        else if (!line.startsWith("--") && line.includes(":")) {
            // Header
            result = result.concat(styled_header(line));
        }
        else {
            // Body
            result = result.concat(styled_body(line));
        }
    }
    return result;
}
function figma_json_to_ftd(json) {
    const cs_data = JSON.parse(json);
    let cs_light = Object.keys(cs_data)
        .filter((key) => key.includes("-light"))
        .reduce((obj, key) => {
        obj = cs_data[key];
        return obj;
    }, {});
    let cs_dark = Object.keys(cs_data)
        .filter((key) => key.includes("-dark"))
        .reduce((obj, key) => {
        obj = cs_data[key];
        return obj;
    }, {});
    let s = `
    -- ftd.color base-:
    light: ${get_color_value(cs_light, "Background Colors", "base")}
    dark: ${get_color_value(cs_dark, "Background Colors", "base")}

    -- ftd.color step-1-:
    light: ${get_color_value(cs_light, "Background Colors", "step-1")}
    dark: ${get_color_value(cs_dark, "Background Colors", "step-1")}

    -- ftd.color step-2-:
    light: ${get_color_value(cs_light, "Background Colors", "step-2")}
    dark: ${get_color_value(cs_dark, "Background Colors", "step-2")}

    -- ftd.color overlay-:
    light: ${get_color_value(cs_light, "Background Colors", "overlay")}
    dark: ${get_color_value(cs_dark, "Background Colors", "overlay")}

    -- ftd.color code-:
    light: ${get_color_value(cs_light, "Background Colors", "code")}
    dark: ${get_color_value(cs_dark, "Background Colors", "code")}

    -- ftd.background-colors background-:
    base: $base-
    step-1: $step-1-
    step-2: $step-2-
    overlay: $overlay-
    code: $code-

    -- ftd.color border-:
    light: ${get_color_value(cs_light, "Standalone Colors", "border")}
    dark: ${get_color_value(cs_dark, "Standalone Colors", "border")}

    -- ftd.color border-strong-:
    light: ${get_color_value(cs_light, "Standalone Colors", "border-strong")}
    dark: ${get_color_value(cs_dark, "Standalone Colors", "border-strong")}

    -- ftd.color text-:
    light: ${get_color_value(cs_light, "Standalone Colors", "text")}
    dark: ${get_color_value(cs_dark, "Standalone Colors", "text")}

    -- ftd.color text-strong-:
    light: ${get_color_value(cs_light, "Standalone Colors", "text-strong")}
    dark: ${get_color_value(cs_dark, "Standalone Colors", "text-strong")}

    -- ftd.color shadow-:
    light: ${get_color_value(cs_light, "Standalone Colors", "shadow")}
    dark: ${get_color_value(cs_dark, "Standalone Colors", "shadow")}

    -- ftd.color scrim-:
    light: ${get_color_value(cs_light, "Standalone Colors", "scrim")}
    dark: ${get_color_value(cs_dark, "Standalone Colors", "scrim")}

    -- ftd.color cta-primary-base-:
    light: ${get_color_value(cs_light, "CTA Primary Colors", "base")}
    dark: ${get_color_value(cs_dark, "CTA Primary Colors", "base")}

    -- ftd.color cta-primary-hover-:
    light: ${get_color_value(cs_light, "CTA Primary Colors", "hover")}
    dark: ${get_color_value(cs_dark, "CTA Primary Colors", "hover")}

    -- ftd.color cta-primary-pressed-:
    light: ${get_color_value(cs_light, "CTA Primary Colors", "pressed")}
    dark: ${get_color_value(cs_dark, "CTA Primary Colors", "pressed")}

    -- ftd.color cta-primary-disabled-:
    light: ${get_color_value(cs_light, "CTA Primary Colors", "disabled")}
    dark: ${get_color_value(cs_dark, "CTA Primary Colors", "disabled")}

    -- ftd.color cta-primary-focused-:
    light: ${get_color_value(cs_light, "CTA Primary Colors", "focused")}
    dark: ${get_color_value(cs_dark, "CTA Primary Colors", "focused")}

    -- ftd.color cta-primary-border-:
    light: ${get_color_value(cs_light, "CTA Primary Colors", "border")}
    dark: ${get_color_value(cs_dark, "CTA Primary Colors", "border")}

    -- ftd.color cta-primary-text-:
    light: ${get_color_value(cs_light, "CTA Primary Colors", "text")}
    dark: ${get_color_value(cs_dark, "CTA Primary Colors", "text")}


    -- ftd.cta-colors cta-primary-:
    base: $cta-primary-base-
    hover: $cta-primary-hover-
    pressed: $cta-primary-pressed-
    disabled: $cta-primary-disabled-
    focused: $cta-primary-focused-
    border: $cta-primary-border-
    text: $cta-primary-text-

    -- ftd.color cta-secondary-base-:
    light: ${get_color_value(cs_light, "CTA Secondary Colors", "base")}
    dark: ${get_color_value(cs_dark, "CTA Secondary Colors", "base")}

    -- ftd.color cta-secondary-hover-:
    light: ${get_color_value(cs_light, "CTA Secondary Colors", "hover")}
    dark: ${get_color_value(cs_dark, "CTA Secondary Colors", "hover")}

    -- ftd.color cta-secondary-pressed-:
    light: ${get_color_value(cs_light, "CTA Secondary Colors", "pressed")}
    dark: ${get_color_value(cs_dark, "CTA Secondary Colors", "pressed")}

    -- ftd.color cta-secondary-disabled-:
    light: ${get_color_value(cs_light, "CTA Secondary Colors", "disabled")}
    dark: ${get_color_value(cs_dark, "CTA Secondary Colors", "disabled")}

    -- ftd.color cta-secondary-focused-:
    light: ${get_color_value(cs_light, "CTA Secondary Colors", "focused")}
    dark: ${get_color_value(cs_dark, "CTA Secondary Colors", "focused")}

    -- ftd.color cta-secondary-border-:
    light: ${get_color_value(cs_light, "CTA Secondary Colors", "border")}
    dark: ${get_color_value(cs_dark, "CTA Secondary Colors", "border")}

    -- ftd.color cta-secondary-text-:
    light: ${get_color_value(cs_light, "CTA Secondary Colors", "text")}
    dark: ${get_color_value(cs_dark, "CTA Secondary Colors", "text")}

    -- ftd.cta-colors cta-secondary-:
    base: $cta-secondary-base-
    hover: $cta-secondary-hover-
    pressed: $cta-secondary-pressed-
    disabled: $cta-secondary-disabled-
    focused: $cta-secondary-focused-
    border: $cta-secondary-border-
    text: $cta-secondary-text-

    -- ftd.color cta-tertiary-base-:
    light: ${get_color_value(cs_light, "CTA Tertiary Colors", "base")}
    dark: ${get_color_value(cs_dark, "CTA Tertiary Colors", "base")}

    -- ftd.color cta-tertiary-hover-:
    light: ${get_color_value(cs_light, "CTA Tertiary Colors", "hover")}
    dark: ${get_color_value(cs_dark, "CTA Tertiary Colors", "hover")}

    -- ftd.color cta-tertiary-pressed-:
    light: ${get_color_value(cs_light, "CTA Tertiary Colors", "pressed")}
    dark: ${get_color_value(cs_dark, "CTA Tertiary Colors", "pressed")}

    -- ftd.color cta-tertiary-disabled-:
    light: ${get_color_value(cs_light, "CTA Tertiary Colors", "disabled")}
    dark: ${get_color_value(cs_dark, "CTA Tertiary Colors", "disabled")}

    -- ftd.color cta-tertiary-focused-:
    light: ${get_color_value(cs_light, "CTA Tertiary Colors", "focused")}
    dark: ${get_color_value(cs_dark, "CTA Tertiary Colors", "focused")}

    -- ftd.color cta-tertiary-border-:
    light: ${get_color_value(cs_light, "CTA Tertiary Colors", "border")}
    dark: ${get_color_value(cs_dark, "CTA Tertiary Colors", "border")}

    -- ftd.color cta-tertiary-text-:
    light: ${get_color_value(cs_light, "CTA Tertiary Colors", "text")}
    dark: ${get_color_value(cs_dark, "CTA Tertiary Colors", "text")}

    -- ftd.cta-colors cta-tertiary-:
    base: $cta-tertiary-base-
    hover: $cta-tertiary-hover-
    pressed: $cta-tertiary-pressed-
    disabled: $cta-tertiary-disabled-
    focused: $cta-tertiary-focused-
    border: $cta-tertiary-border-
    text: $cta-tertiary-text-

    -- ftd.color cta-danger-base-:
    light: ${get_color_value(cs_light, "CTA Danger Colors", "base")}
    dark: ${get_color_value(cs_dark, "CTA Danger Colors", "base")}

    -- ftd.color cta-danger-hover-:
    light: ${get_color_value(cs_light, "CTA Danger Colors", "hover")}
    dark: ${get_color_value(cs_dark, "CTA Danger Colors", "hover")}

    -- ftd.color cta-danger-pressed-:
    light: ${get_color_value(cs_light, "CTA Danger Colors", "pressed")}
    dark: ${get_color_value(cs_dark, "CTA Danger Colors", "pressed")}

    -- ftd.color cta-danger-disabled-:
    light: ${get_color_value(cs_light, "CTA Danger Colors", "disabled")}
    dark: ${get_color_value(cs_dark, "CTA Danger Colors", "disabled")}

    -- ftd.color cta-danger-focused-:
    light: ${get_color_value(cs_light, "CTA Danger Colors", "focused")}
    dark: ${get_color_value(cs_dark, "CTA Danger Colors", "focused")}

    -- ftd.color cta-danger-border-:
    light: ${get_color_value(cs_light, "CTA Danger Colors", "border")}
    dark: ${get_color_value(cs_dark, "CTA Danger Colors", "border")}

    -- ftd.color cta-danger-text-:
    light: ${get_color_value(cs_light, "CTA Danger Colors", "text")}
    dark: ${get_color_value(cs_dark, "CTA Danger Colors", "text")}

    -- ftd.cta-colors cta-danger-:
    base: $cta-danger-base-
    hover: $cta-danger-hover-
    pressed: $cta-danger-pressed-
    disabled: $cta-danger-disabled-
    focused: $cta-danger-focused-
    border: $cta-danger-border-
    text: $cta-danger-text-

    -- ftd.color accent-primary-:
    light: ${get_color_value(cs_light, "Accent Colors", "primary")}
    dark: ${get_color_value(cs_dark, "Accent Colors", "primary")}

    -- ftd.color accent-secondary-:
    light: ${get_color_value(cs_light, "Accent Colors", "secondary")}
    dark: ${get_color_value(cs_dark, "Accent Colors", "secondary")}

    -- ftd.color accent-tertiary-:
    light: ${get_color_value(cs_light, "Accent Colors", "tertiary")}
    dark: ${get_color_value(cs_dark, "Accent Colors", "tertiary")}

    -- ftd.pst accent-:
    primary: $accent-primary-
    secondary: $accent-secondary-
    tertiary: $accent-tertiary-

    -- ftd.color error-base-:
    light: ${get_color_value(cs_light, "Error Colors", "base")}
    dark: ${get_color_value(cs_dark, "Error Colors", "base")}

    -- ftd.color error-text-:
    light: ${get_color_value(cs_light, "Error Colors", "text")}
    dark: ${get_color_value(cs_dark, "Error Colors", "text")}

    -- ftd.color error-border-:
    light: ${get_color_value(cs_light, "Error Colors", "border")}
    dark: ${get_color_value(cs_dark, "Error Colors", "border")}

    -- ftd.btb error-btb-:
    base: $error-base-
    text: $error-text-
    border: $error-border-

    -- ftd.color success-base-:
    light: ${get_color_value(cs_light, "Success Colors", "base")}
    dark: ${get_color_value(cs_dark, "Success Colors", "base")}

    -- ftd.color success-text-:
    light: ${get_color_value(cs_light, "Success Colors", "text")}
    dark: ${get_color_value(cs_dark, "Success Colors", "text")}

    -- ftd.color success-border-:
    light: ${get_color_value(cs_light, "Success Colors", "border")}
    dark: ${get_color_value(cs_dark, "Success Colors", "border")}

    -- ftd.btb success-btb-:
    base: $success-base-
    text: $success-text-
    border: $success-border-

    -- ftd.color info-base-:
    light: ${get_color_value(cs_light, "Info Colors", "base")}
    dark: ${get_color_value(cs_dark, "Info Colors", "base")}

    -- ftd.color info-text-:
    light: ${get_color_value(cs_light, "Info Colors", "text")}
    dark: ${get_color_value(cs_dark, "Info Colors", "text")}

    -- ftd.color info-border-:
    light: ${get_color_value(cs_light, "Info Colors", "border")}
    dark: ${get_color_value(cs_dark, "Info Colors", "border")}

    -- ftd.btb info-btb-:
    base: $info-base-
    text: $info-text-
    border: $info-border-

    -- ftd.color warning-base-:
    light: ${get_color_value(cs_light, "Warning Colors", "base")}
    dark: ${get_color_value(cs_dark, "Warning Colors", "base")}

    -- ftd.color warning-text-:
    light: ${get_color_value(cs_light, "Warning Colors", "text")}
    dark: ${get_color_value(cs_dark, "Warning Colors", "text")}

    -- ftd.color warning-border-:
    light: ${get_color_value(cs_light, "Warning Colors", "border")}
    dark: ${get_color_value(cs_dark, "Warning Colors", "border")}

    -- ftd.btb warning-btb-:
    base: $warning-base-
    text: $warning-text-
    border: $warning-border-

    -- ftd.color custom-one-:
    light: ${get_color_value(cs_light, "Custom Colors", "one")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "one")}

    -- ftd.color custom-two-:
    light: ${get_color_value(cs_light, "Custom Colors", "two")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "two")}

    -- ftd.color custom-three-:
    light: ${get_color_value(cs_light, "Custom Colors", "three")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "three")}

    -- ftd.color custom-four-:
    light: ${get_color_value(cs_light, "Custom Colors", "four")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "four")}

    -- ftd.color custom-five-:
    light: ${get_color_value(cs_light, "Custom Colors", "five")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "five")}

    -- ftd.color custom-six-:
    light: ${get_color_value(cs_light, "Custom Colors", "six")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "six")}

    -- ftd.color custom-seven-:
    light: ${get_color_value(cs_light, "Custom Colors", "seven")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "seven")}

    -- ftd.color custom-eight-:
    light: ${get_color_value(cs_light, "Custom Colors", "eight")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "eight")}

    -- ftd.color custom-nine-:
    light: ${get_color_value(cs_light, "Custom Colors", "nine")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "nine")}

    -- ftd.color custom-ten-:
    light: ${get_color_value(cs_light, "Custom Colors", "ten")}
    dark: ${get_color_value(cs_dark, "Custom Colors", "ten")}

    -- ftd.custom-colors custom-:
    one: $custom-one-
    two: $custom-two-
    three: $custom-three-
    four: $custom-four-
    five: $custom-five-
    six: $custom-six-
    seven: $custom-seven-
    eight: $custom-eight-
    nine: $custom-nine-
    ten: $custom-ten-

    -- ftd.color-scheme main:
    background: $background-
    border: $border-
    border-strong: $border-strong-
    text: $text-
    text-strong: $text-strong-
    shadow: $shadow-
    scrim: $scrim-
    cta-primary: $cta-primary-
    cta-secondary: $cta-secondary-
    cta-tertiary: $cta-tertiary-
    cta-danger: $cta-danger-
    accent: $accent-
    error: $error-btb-
    success: $success-btb-
    info: $info-btb-
    warning: $warning-btb-
    custom: $custom-
    `;
    let fs = `<pre>${apply_style(s)}</pre>`;
    return [s, fs];
}
function len(data) {
    return data.length;
}
function fallbackCopyTextToClipboard(text) {
    var textArea = document.createElement("textarea");
    textArea.value = text;
    // Avoid scrolling to bottom
    textArea.style.top = "0";
    textArea.style.left = "0";
    textArea.style.position = "fixed";
    document.body.appendChild(textArea);
    textArea.focus();
    textArea.select();
    try {
        var successful = document.execCommand('copy');
        var msg = successful ? 'successful' : 'unsuccessful';
        console.log('Fallback: Copying text command was ' + msg);
    }
    catch (err) {
        console.error('Fallback: Oops, unable to copy', err);
    }
    document.body.removeChild(textArea);
}