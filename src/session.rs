use crate::channel;
use crate::channel::ChannelRelay;
use crate::errors::*;
use actix::*;
use actix::{Actor, StreamHandler};
use actix_rt::spawn;
use actix_web_actors::ws;
use async_trait::async_trait;
use bytes::Bytes;
use std::sync::Arc;
use std::time::{Duration, Instant};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

#[async_trait]
pub trait SessionHook: Sync {
    async fn on_connect_broadcast(
        &self,
    ) -> Result<(Option<String>, Option<bytes::Bytes>), ChannelError> {
        Ok((None, None))
    }

    async fn client_to_channel_text(&self, text: String) -> Result<String, ChannelError> {
        Ok(text)
    }

    async fn client_to_channel_binary(
        &self,
        bytes: bytes::Bytes,
    ) -> Result<bytes::Bytes, ChannelError> {
        Ok(bytes)
    }

    fn channel_to_client_text(&self, text: Arc<String>) -> Result<String, ChannelError> {
        Ok(text.to_string())
    }

    fn channel_to_client_binary(
        &self,
        bytes: Arc<bytes::Bytes>,
    ) -> Result<Arc<bytes::Bytes>, ChannelError> {
        Ok(bytes)
    }
}

/// Define WS actor
pub struct ChannelSession {
    id: crate::SessionId,
    channel_id: crate::ChannelId,
    channel: String,
    hb: Instant,
    relay_addr: Addr<ChannelRelay>,
    hooks: Arc<Box<dyn SessionHook>>,
}

impl ChannelSession {
    pub fn new(channel: &str, relay: Addr<ChannelRelay>, hooks: Box<dyn SessionHook>) -> Self {
        Self {
            id: 0,
            channel_id: 0,
            hb: Instant::now(),
            channel: channel.to_owned(),
            relay_addr: relay,
            hooks: Arc::new(hooks),
        }
    }

    // Inform the Relay we are dying
    pub fn unsubscribe(&self) {
        let msg = channel::UnsubscribeAll(self.id);
        self.relay_addr.do_send(msg);
    }

    fn subscribe(&self, ctx: &mut ws::WebsocketContext<Self>) {
        if self.channel == "" {
            //already subscribed
            return;
        };
        let addr_text = ctx.address();
        let addr_bin = ctx.address();
        let msg = channel::Subscribe {
            channel: self.channel.clone(),
            addr_text: addr_text.recipient(),
            addr_bin: addr_bin.recipient(),
        };
        // try to send the message to subscribe, if it fails kill the actor
        self.relay_addr
            .send(msg)
            .into_actor(self)
            .then(|res, act, ctx| {
                match res {
                    Ok(m_res) => {
                        act.id = m_res.session_id;
                        act.channel_id = m_res.channel_id;
                        act.channel = "".to_owned();

                        // Handle the on connect hook
                        let hooks = act.hooks.clone();
                        let channel_id = act.channel_id;
                        let self_id = 1; //message from server NOT user
                        let addr = act.relay_addr.clone();
                        let self_addr = ctx.address();
                        spawn(async move {
                            handle_client_connect(hooks, channel_id, self_id, addr, self_addr)
                                .await;
                        });
                    }
                    // something is wrong with chat server
                    _ => ctx.stop(),
                }
                fut::ready(())
            })
            .wait(ctx);
    }

    /// helper method that sends ping to client every second.
    fn start_heartbeat(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats
            if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
                // heartbeat timed out
                log::debug!("WS heartbeat failed, disconnecting :{}", act.id);
                act.unsubscribe();
                // stop actor
                ctx.stop();
                // don't try to send a ping
                return;
            }
            ctx.ping(b"");
        });
    }
}

impl Actor for ChannelSession {
    type Context = ws::WebsocketContext<Self>;
    // Method is called on actor start.
    fn started(&mut self, ctx: &mut Self::Context) {
        log::debug!("STARTED: {}", self.id);
        self.start_heartbeat(ctx);
        self.subscribe(ctx);
        log::debug!("CONNECTED: {}", self.id);
    }
}

/// Handle messages from channel, we simply send it to peer websocket
impl Handler<channel::BroadcastText> for ChannelSession {
    type Result = ();
    fn handle(&mut self, msg: channel::BroadcastText, ctx: &mut Self::Context) {
        if msg.1 == self.id {
            //don't repeat the msg back to yourself
            return;
        }
        let hooks = self.hooks.clone();
        //let hooks = hooks.lock().unwrap();
        if let Ok(text) = hooks.channel_to_client_text(msg.2) {
            ctx.text(text);
        }
    }
}

/// Handle messages from channel, we simply send it to peer websocket
impl Handler<channel::BroadcastBytes> for ChannelSession {
    type Result = ();
    fn handle(&mut self, msg: channel::BroadcastBytes, ctx: &mut Self::Context) {
        if msg.1 == self.id {
            //don't repeat the msg back to yourself
            return;
        }
        let hooks = self.hooks.clone();
        //let hooks = hooks.lock().unwrap();
        if let Ok(raw) = hooks.channel_to_client_binary(msg.2) {
            let bytes: &[u8] = &raw;
            let bytes = Bytes::copy_from_slice(bytes);
            ctx.binary(bytes);
        }
    }
}

#[derive(actix::prelude::Message)]
#[rtype(result = "()")]
enum AsyncDone {
    RespondText(String),
    RespondBinary(bytes::Bytes),
}
impl Handler<AsyncDone> for ChannelSession {
    type Result = ();
    fn handle(&mut self, done: AsyncDone, ctx: &mut Self::Context) -> Self::Result {
        match done {
            AsyncDone::RespondText(text) => ctx.text(text),
            AsyncDone::RespondBinary(bin) => ctx.binary(bin),
        }
        return ();
    }
}

//Call the hook to see what the developer wants to do with this incoming message
//relay the output accordingly
async fn handle_client_text(
    client_text: String,
    hooks: Arc<Box<dyn SessionHook>>,
    channel_id: crate::ChannelId,
    self_id: crate::SessionId,
    relay_addr: Addr<ChannelRelay>,
    self_addr: Addr<ChannelSession>,
) {
    let result = {
        let hooks = hooks.clone();
        hooks.client_to_channel_text(client_text).await
    };
    match result {
        Ok(text) => {
            let msg = channel::BroadcastText(channel_id, self_id, Arc::new(text));
            let _ = relay_addr.send(msg).await;
        }
        Err(err) => handle_error(err, self_addr).await,
    }
}

//Call the hook to see what the developer wants to do with this incoming message
//relay the output accordingly
async fn handle_client_binary(
    client_bin: bytes::Bytes,
    hooks: Arc<Box<dyn SessionHook>>,
    channel_id: crate::ChannelId,
    self_id: crate::SessionId,
    relay_addr: Addr<ChannelRelay>,
    self_addr: Addr<ChannelSession>,
) {
    let result = {
        let hooks = hooks.clone();
        hooks.client_to_channel_binary(client_bin).await
    };
    match result {
        Ok(bin) => {
            let msg = channel::BroadcastBytes(channel_id, self_id, Arc::new(bin));
            let _ = relay_addr.send(msg).await;
        }
        Err(err) => handle_error(err, self_addr).await,
    }
}

//The hook has returned a Err. We need to handle it in the appropriate manner
async fn handle_error(err: ChannelError, self_addr: Addr<ChannelSession>) {
    match err {
        ChannelError::StopMessage => return,
        ChannelError::SendClientText(to_client) => {
            let msg = AsyncDone::RespondText(to_client);
            let _ = self_addr.send(msg).await;
        }
        ChannelError::SendClientBinary(to_client) => {
            let msg = AsyncDone::RespondBinary(to_client);
            let _ = self_addr.send(msg).await;
        }
    }
}

// Handle the on connect hook
async fn handle_client_connect(
    hooks: Arc<Box<dyn SessionHook>>,
    channel_id: crate::ChannelId,
    self_id: crate::SessionId,
    relay_addr: Addr<ChannelRelay>,
    self_addr: Addr<ChannelSession>,
) {
    let result = {
        let hooks = hooks.clone();
        hooks.on_connect_broadcast().await
    };
    // Handle any error from the hooks if there are any
    let (to_channel_text, to_channel_bin) = match result {
        Ok(to_channel) => to_channel,
        Err(err) => return handle_error(err, self_addr).await,
    };
    // if hook text to broadcast, send it to the relay
    if let Some(text) = to_channel_text {
        let msg = channel::BroadcastText(channel_id, self_id, Arc::new(text));
        relay_addr.do_send(msg);
    }
    // if hook bin to broadcast, send it to the relay
    if let Some(bin) = to_channel_bin {
        let msg = channel::BroadcastBytes(channel_id, self_id, Arc::new(bin));
        relay_addr.do_send(msg);
    }
}

// Handler for ws::Message message
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ChannelSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                log::debug!("Ping: {:?}", msg);
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(msg)) => {
                log::debug!("Pong: {:?}", msg);
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(client_text)) => {
                log::debug!(
                    "WS from:{}, to_channel: {}, text: {}",
                    self.id,
                    self.channel_id,
                    &client_text
                );

                let hooks = self.hooks.clone();
                let channel_id = self.channel_id;
                let self_id = self.id;
                let addr = self.relay_addr.clone();
                let self_addr = ctx.address();
                spawn(async move {
                    handle_client_text(client_text, hooks, channel_id, self_id, addr, self_addr)
                        .await;
                });
            }
            Ok(ws::Message::Binary(client_bin)) => {
                log::debug!(
                    "WS from:{}, to_channel: {}, bytes: {}",
                    self.id,
                    self.channel_id,
                    client_bin.len()
                );

                let hooks = self.hooks.clone();
                let channel_id = self.channel_id;
                let self_id = self.id;
                let addr = self.relay_addr.clone();
                let self_addr = ctx.address();
                spawn(async move {
                    handle_client_binary(client_bin, hooks, channel_id, self_id, addr, self_addr)
                        .await;
                });
            }
            Ok(ws::Message::Close(reason)) => {
                self.unsubscribe();
                ctx.close(reason);
                ctx.stop();
                log::debug!("DISCONNECTED: {}", self.id);
            }
            _ => {
                log::debug!("OTHER MESSAGE: {:?}", msg);
            }
        }
    }
}
