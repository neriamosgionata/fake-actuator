use std::net::SocketAddr;
use coap_lite::{CoapRequest, RequestType};
use serde_json::json;
use local_ip_address::local_ip;
use fake_actuator::{CoAPClient, Server};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let url_register = "coap://127.0.0.1:5683/actuator/register";

    let mut actuator_ip_address = String::new();
    let actuator_port = 5684i16;

    match local_ip() {
        Ok(ip) => {
            actuator_ip_address.push_str(ip.to_string().as_str());
        }
        Err(_) => panic!("Unable to get local IP address")
    }

    println!("Local IP address: {}", actuator_ip_address);

    let register_params = json! {
        {
            "ip_address": actuator_ip_address,
            "port": actuator_port,
        }
    }.to_string().as_bytes().to_vec();

    let response_register = CoAPClient::post(url_register, register_params).unwrap();
    let new_actuator = String::from_utf8(response_register.message.payload).unwrap();

    if new_actuator == "KO" {
        println!("Error registering actuator");
        return Ok(());
    }

    let _actuator_id = new_actuator.parse::<i32>().unwrap();

    run_server(actuator_port).await;

    Ok(())
}

async fn run_server(actuator_port: i16) {
    println!("Running server");

    let address = "127.0.0.1:".to_owned() + actuator_port.to_string().as_str();

    let mut server = Server::new(address).unwrap();

    server.run(
        |request| async move {
            let request_ref = &request;

            let payload = callback(request_ref).await;

            match request.response {
                Some(mut message) => {
                    message.message.payload = payload.as_bytes().to_vec();

                    Some(message)
                }
                _ => None,
            }
        },
    )
        .await
        .expect("Failed to create server");
}

async fn callback(request: &CoapRequest<SocketAddr>) -> String {
    println!("Callback called");

    let payload = String::from_utf8(request.message.payload.clone()).unwrap();

    if request.get_method() != &RequestType::Post {
        println!("Not a POST request");
        return "KO".to_string();
    }

    println!("POST request");

    if payload == "ON" {
        println!("ON");
        "ON".to_string()
    } else if payload == "OFF" {
        println!("OFF");
        "OFF".to_string()
    } else {
        println!("Unknown command");
        "KO".to_string()
    }
}