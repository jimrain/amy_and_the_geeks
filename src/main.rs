//! Default Compute@Edge template program.

use fastly::http::{header, HeaderValue, Method, StatusCode};
use fastly::{mime, Dictionary, Error, Request, Response};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::collections::HashMap;

/// The name of a backend server associated with this service.
///
/// This should be changed to match the name of your own backend. See the the `Hosts` section of
/// the Fastly WASM service UI for more information.
const FASTLY_API_BACKEND_NAME: &str = "fastly_api_backend";

const FASTLY_API_BASE: &str = "https://api.fastly.com";

const FASTLY_API_DATACENTER_ENDPOINT: &str = "https://api.fastly.com/datacenters";

/// The name of a second backend associated with this service.
const POP_STATUS_API_BACKEND_NAME: &str = "pop_status_backend";

const POP_STATUS_API_ENDPOINT: &str = "https://service-scraper.edgecompute.app/";

// JMR - put this in an encrypted dictionary!
// const FSLY_API_TOKEN: &str = "Y3woXFscylfKhZvGC3rS-1OJqp8HtZjs";
const FSLY_API_TOKEN: &str = "ewhqN789jdp625r_DUgYaqjvuf6Cb6hP";

const APP_DATA_DICT: &str = "app_data";

const STATUS_VALUES: &'static [&'static str] = &[
    "Operational",
    "Degraded Performance",
    "Partial Outage",
    "Major Outage",
    "Maintenance",
    "Not Available",
];

#[derive(Serialize, Deserialize, Debug)]
struct Coordinates {
    x: u32,
    y: u32,
    latitude: f64,
    longitude: f64,
}

#[derive(Serialize, Deserialize, Debug)]
struct PopData {
    code: String,
    name: String,
    group: String,
    coordinates: Coordinates,
    shield: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct StatusData {
    code: String,
    status: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct PopStatusData {
    code: String,
    name: String,
    latitude: f64,
    longitude: f64,
    group: String,
    shield: String,
    status: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct PopStatusResponse {
    current_pop: String,
    pop_status_data: Vec<PopStatusData>,
}

#[derive(Serialize, Deserialize, Debug)]
struct DictionaryInfo {
    dictionary_id: String,
    service_id: String,
    item_key: String,
    item_value: String,
}

/// The entry point for your application.
///
/// This function is triggered when your service receives a client request. It could be used to
/// route based on the request properties (such as method or path), send the request to a backend,
/// make completely new requests, and/or generate synthetic responses.
///
/// If `main` returns an error, a 500 error response will be delivered to the client.
#[fastly::main]
fn main(mut req: Request) -> Result<Response, Error> {
    println!(
        "Amy and the Geeks version:{}",
        std::env::var("FASTLY_SERVICE_VERSION").unwrap_or_else(|_| String::new())
    );

    let current_pop = std::env::var("FASTLY_POP").unwrap_or_else(|_| String::new());
    println!("Current:{}", current_pop);

    // Filter request methods...
    match req.get_method() {
        // Allow GET and HEAD requests.
        &Method::GET | &Method::HEAD | &Method::PUT => (),

        // Accept PURGE requests; it does not matter to which backend they are sent.
        m if m == "PURGE" => (),

        // Deny anything else.
        _ => {
            return Ok(Response::from_status(StatusCode::METHOD_NOT_ALLOWED)
                .with_header(header::ALLOW, "GET, HEAD")
                .with_body_text_plain("This method is not allowed\n"))
        }
    };

    let app_data_dict = Dictionary::open(APP_DATA_DICT);
    let service_id = std::env::var("FASTLY_SERVICE_ID").unwrap_or_else(|_| String::new());
    // We need the dictionary id for API calls.
    let dict_id = app_data_dict.get("dict_id").unwrap();

    let the_path = req.get_path();
    println!("Path: {}", the_path);
    // Pattern match on the path.
    match the_path {
        // If request is to the `/` path, send a default response.
        "/" | "/noscrape" => {
            let pop_response = Request::new(Method::GET, FASTLY_API_DATACENTER_ENDPOINT)
                .with_header("Fastly-Key", FSLY_API_TOKEN)
                .with_header(header::ACCEPT, "application/json")
                .send(FASTLY_API_BACKEND_NAME)?;

            let body_str = pop_response.into_body_str();
            let pop_vec: Vec<PopData> = serde_json::from_str(&body_str).unwrap();

            let mut status_map: Option<HashMap<&str, &str>> = None;
            let mut status_vec: Vec<StatusData>;
            if the_path != "/noscrape" {
                let status_response = Request::new(Method::GET, POP_STATUS_API_ENDPOINT)
                    .with_header(header::ACCEPT, "application/json")
                    .send(POP_STATUS_API_BACKEND_NAME)?;

                println!("Status response: {:?}", status_response.get_status());

                let status_body_str = status_response.into_body_str();
                // println!("Status body: {}", &status_body_str);

                status_vec = serde_json::from_str(&status_body_str).unwrap();

                status_map = Some(
                    status_vec
                        .iter()
                        .map(|status| (status.code.as_str(), status.status.as_str()))
                        .collect(),
                );
            }

            // let modified_pop_status = app_data_dict.get("modified_pop_status").unwrap();
            let modified_pop_status_opt= get_modified_pop_status(&service_id, &dict_id);
            if modified_pop_status_opt.is_none() {
                return Ok(Response::from_status(StatusCode::IM_A_TEAPOT)
                    .with_body_text_plain("Problem accessing API\n"));
            }

            let modified_pop_status= modified_pop_status_opt.unwrap();
            let modified_pop_status_map: HashMap<&str, u8> =
                serde_json::from_str(modified_pop_status.as_str()).unwrap();

            let pop_status_vec: Vec<PopStatusData> = pop_vec
                .iter()
                .map(|pop| {
                    let pop_code = pop.code.to_string();
                    let status = get_pop_status(&pop_code, &status_map, &modified_pop_status_map);
                    let shield = match &pop.shield {
                        Some(s) => s,
                        None => "",
                    };

                    PopStatusData {
                        code: pop_code,
                        name: pop.name.to_string(),
                        latitude: pop.coordinates.latitude,
                        longitude: pop.coordinates.longitude,
                        group: pop.group.to_string(),
                        shield: shield.to_string(),
                        status: status,
                    }
                })
                .collect();

            let pop_status_response: PopStatusResponse = PopStatusResponse {
                current_pop: current_pop,
                pop_status_data: pop_status_vec,
            };

            let pop_status_json = serde_json::to_string(&pop_status_response)?;

            Ok(Response::from_status(StatusCode::OK)
                .with_content_type(mime::APPLICATION_JSON)
                .with_header(
                    &header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    &HeaderValue::from_static("*"),
                )
                .with_body(pop_status_json))
        }

        "/set_pop" => {
            let mut modified_pop_status_opt= get_modified_pop_status(&service_id, &dict_id);
            if modified_pop_status_opt.is_none() {
                return Ok(Response::from_status(StatusCode::IM_A_TEAPOT)
                    .with_body_text_plain("Problem accessing API\n"));
            }

            let modified_pop_status= modified_pop_status_opt.unwrap();
            let mut modified_pop_status_map: HashMap<String, u8> =
                serde_json::from_str(modified_pop_status.as_str()).unwrap();

            let query_params: Vec<(String, String)> = req.get_query().unwrap();
            println!("QP: {:?}", query_params);

            if query_params.is_empty() {
                let response = Response::from_body(modified_pop_status)
                    .with_status(StatusCode::OK)
                    .with_content_type(mime::APPLICATION_JSON)
                    .with_header(
                        &header::ACCESS_CONTROL_ALLOW_ORIGIN,
                        &HeaderValue::from_static("*"),
                    );
                return Ok(response);
            }

            for (pop, status) in query_params {
                if pop == "*" {
                    if status == "-" {
                        modified_pop_status_map.clear();
                    } else {
                        modified_pop_status_map
                            .insert("*".to_string(), status.parse::<u8>().unwrap());
                    }
                } else {
                    if status == "-" {
                        modified_pop_status_map.remove(pop.as_str());
                    } else {
                        modified_pop_status_map.insert(pop, status.parse::<u8>().unwrap());
                    }
                }
            }

            // /service/service_id/dictionary/dictionary_id/item/dictionary_item_key
            let the_url = format!(
                "{}/service/{}/dictionary/{}/item/modified_pop_status",
                FASTLY_API_BASE, service_id, dict_id
            );
            let the_body = format!(
                "item_value={}",
                serde_json::to_string(&modified_pop_status_map)?
            );
            let mut dict_api_response = Request::new(Method::PUT, the_url)
                .with_header("Fastly-Key", FSLY_API_TOKEN)
                .with_header(header::ACCEPT, "application/json")
                .with_header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .with_body(the_body)
                .send(FASTLY_API_BACKEND_NAME)?;

            if dict_api_response.get_status() == StatusCode::OK {
                let body_str = dict_api_response.into_body_str();
                let dict_info: DictionaryInfo = serde_json::from_str(&body_str).unwrap();
                Ok(Response::from_status(StatusCode::OK)
                    .with_content_type(mime::APPLICATION_JSON)
                    .with_header(
                        &header::ACCESS_CONTROL_ALLOW_ORIGIN,
                        &HeaderValue::from_static("*"),
                    )
                    .with_body(dict_info.item_value))
            } else {
                Ok(Response::from_status(StatusCode::IM_A_TEAPOT)
                    .with_body_text_plain("Problem mofifying dictionary\n"))
            }
        }

        // Catch all other requests and return a 404.
        _ => Ok(Response::from_status(StatusCode::NOT_FOUND)
            .with_body_text_plain("The page you requested could not be found\n")),
    }
}

fn get_pop_status(
    pop_code: &str,
    status_map: &Option<HashMap<&str, &str>>,
    modified_pop_status_vec: &HashMap<&str, u8>,
) -> String {
    if modified_pop_status_vec.contains_key("*") {
        let pc_index = modified_pop_status_vec["*"];
        if pc_index < STATUS_VALUES.len() as u8 {
            STATUS_VALUES[pc_index as usize].to_string()
        } else {
            get_status_from_map(pop_code, status_map)
        }
    } else {
        match modified_pop_status_vec.get(pop_code) {
            Some(pc_index) => STATUS_VALUES[*pc_index as usize].to_string(),
            None => get_status_from_map(pop_code, status_map),
        }
    }
}

fn get_status_from_map(pop_code: &str, status_map: &Option<HashMap<&str, &str>>) -> String {
    match status_map {
        Some(map) => match map.get(pop_code) {
            Some(status) => status.parse().unwrap(),
            None => "Not Available".to_string(),
        },
        None => "Not Available".to_string(),
    }
}

// This is calling the Fastly API to get the dictionary. You might ask why I'm not just accessing
// it on the edge. Reason being to avoid a race where we read it on the edge then write it with the
// API. Still not ideal as there could be a race with another pop but it will do until we have a
// KV store
fn get_modified_pop_status(service_id: &str, dict_id: &str) -> Option<String> {
    let dict_item_url = format!(
        "{}/service/{}/dictionary/{}/item/modified_pop_status",
        FASTLY_API_BASE, service_id, dict_id
    );
    // let modified_pop_status = app_data_dict.get("modified_pop_status").unwrap();
    let mut modified_pop_status_resp = Request::new(Method::GET, dict_item_url)
        .with_header("Fastly-Key", FSLY_API_TOKEN)
        .with_header(header::ACCEPT, "application/json")
        .send(FASTLY_API_BACKEND_NAME).unwrap();

    if modified_pop_status_resp.get_status() == StatusCode::OK {
        let body_str = modified_pop_status_resp.into_body_str();
        let dict_info: DictionaryInfo = serde_json::from_str(&body_str).unwrap();
        let modified_pop_status = dict_info.item_value;
        println!("MPS: {}", modified_pop_status);
        Some(modified_pop_status)
    } else {
        None
    }
}
