
# actix-channels - a wrapper around actix websockets to communicate with multiple people at once.

This crate is indented to be used in with actix-web 

## How to use:

You will need to include in your Cargo.toml
- actix-web = "4.0.1"
- actix = "^0.13"
- actix-web-actors = "4.0.1"
- actix_channels


Simple example
```

use actix::*;
use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use std::env;
use std::sync::Arc;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();

    let port = env::var("PORT").unwrap_or_else(|_| "5000".to_owned());
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_owned());
    let bind_interface: String = format!("{}:{}", host, port);
    log::info!("Server Running: {}", &bind_interface);

    //This is an actix actor
    //boot up a single actor to handle message passing for ws channels
    let ws_relay = actix_channels::ChannelRelay::new().start();

    HttpServer::new(move || {
        App::new()
            .data(ws_relay.clone())
            .service(channel_example)
            .default_service(web::get().to(p404))
    })
    .bind(bind_interface)?
    .run()
    .await
}

pub async fn p404() -> HttpResponse {
    HttpResponse::NotFound().finish()
}

#[get("/channel_example")]
pub async fn channel_example(
    req: HttpRequest,
    stream: web::Payload,
    relay: web::Data<Addr<ChannelRelay>>,
) -> HttpResponse {
    let channel = "/channel_example/itentifyer_of_channel".to_string();
    let hooks = Box::new(ChatRoomHooks {
        chatroom_name: channel.clone(),
    });
    let session = ChannelSession::new(&channel, relay.get_ref().clone(), hooks);
    let resp = ws::start(session, &req, stream);
    match resp {
        Ok(r) => r,
        Err(err) => {
            log::error!("SOCKET ERROR: {:?}", err);
            HttpResponse::build(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR).finish()
        }
    }
}

struct ChatRoomHooks {
    chatroom_name: String,
}

use actix_channels::{
    async_trait, bytes, errors::ChannelError, ChannelRelay, ChannelSession, Dest, SessionHook,
};

#[async_trait]
impl SessionHook for ChatRoomHooks {
    // when a user connects to the channel to this...
    async fn on_connect_broadcast(
        &self,
    ) -> Result<(Option<String>, Option<bytes::Bytes>, Dest), ChannelError> {
        let msg = format!("Hello you just joined: {}", self.chatroom_name);
        Ok((Some(msg), None, Dest::Everyone))
    }

    async fn client_to_channel_text(&self, text: String) -> Result<(String, Dest), ChannelError> {
        // Dest::Others will send to everyone except the sender
        Ok((text.into(), Dest::Others))
    }

    async fn client_to_channel_binary(
        &self,
        _bytes: bytes::Bytes,
    ) -> Result<(bytes::Bytes, Dest), ChannelError> {
        // if you want to block the message return a StopMessage
        Err(ChannelError::StopMessage)
    }

    fn channel_to_client_text(&self, text: Arc<String>) -> Result<String, ChannelError> {
        Ok(text.to_string())
    }

    fn channel_to_client_binary(
        &self,
        _bytes: Arc<bytes::Bytes>,
    ) -> Result<Arc<bytes::Bytes>, ChannelError> {
        // if you want to block the message return a StopMessage
        Err(ChannelError::StopMessage)
    }
}


```



