use actix::prelude::*;
use rand::{self, rngs::ThreadRng, Rng};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::ChannelId;
use crate::SessionId;

use crate::Dest;

pub struct ChannelRelay {
    rng: ThreadRng,
    channels: HashMap<String, ChannelId>,
    sessions: HashMap<SessionId, (Recipient<BroadcastText>, Recipient<BroadcastBytes>)>,
    subscriptions: HashMap<ChannelId, HashSet<SessionId>>,
}

impl ChannelRelay {
    pub fn new() -> Self {
        Self {
            rng: rand::thread_rng(),
            sessions: Default::default(),
            channels: Default::default(),
            subscriptions: Default::default(),
        }
    }
}

impl ChannelRelay {
    fn relay_text_message(&mut self, msg: BroadcastText) -> Option<()> {
        let mut dead: Vec<SessionId> = vec![];
        {
            let subs = self.subscriptions.get(&msg.0)?;
            for session_id in subs {
                match self.sessions.get(session_id) {
                    Some(session) => {
                        let _r = session.0.do_send(msg.clone());
                    }
                    None => dead.push(*session_id),
                }
            }
        }
        //cleanup the dead sessions out of the subscription
        let subs = self.subscriptions.get_mut(&msg.0)?;
        for d in dead {
            subs.remove(&d);
        }
        Some(())
    }

    fn relay_bin_message(&mut self, msg: BroadcastBytes) -> Option<()> {
        let mut dead: Vec<SessionId> = vec![];
        {
            let subs = self.subscriptions.get(&msg.0)?;
            for session_id in subs {
                match self.sessions.get(session_id) {
                    Some(session) => {
                        let _r = session.1.do_send(msg.clone());
                    }
                    None => dead.push(*session_id),
                }
            }
        }
        //cleanup the dead sessions out of the subscription
        let subs = self.subscriptions.get_mut(&msg.0)?;
        for d in dead {
            subs.remove(&d);
        }
        Some(())
    }
}

impl Actor for ChannelRelay {
    type Context = Context<Self>;
}

impl Handler<BroadcastText> for ChannelRelay {
    type Result = ();
    fn handle(&mut self, msg: BroadcastText, _: &mut Context<Self>) -> Self::Result {
        let _ = self.relay_text_message(msg);
    }
}

impl Handler<BroadcastBytes> for ChannelRelay {
    type Result = ();
    fn handle(&mut self, msg: BroadcastBytes, _: &mut Context<Self>) -> Self::Result {
        let _ = self.relay_bin_message(msg);
    }
}

impl Handler<Subscribe> for ChannelRelay {
    type Result = SubscribeResponce;
    fn handle(&mut self, msg: Subscribe, _: &mut Context<Self>) -> Self::Result {
        // register session with random id
        let rng = &mut self.rng;
        let id = rng.gen::<SessionId>();
        let channel = msg.channel;
        self.sessions.insert(id, (msg.addr_text, msg.addr_bin));

        let channel_id = *(self.channels.entry(channel).or_insert_with(|| rng.gen()));

        //get the subs for this channel
        let subs = self
            .subscriptions
            .entry(channel_id)
            .or_insert_with(|| Default::default());

        //add this session_id to the list of subscriptions
        subs.insert(id);
        SubscribeResponce {
            session_id: id,
            channel_id,
        }
    }
}

impl Handler<UnsubscribeAll> for ChannelRelay {
    type Result = ();
    fn handle(&mut self, msg: UnsubscribeAll, _: &mut Context<Self>) -> Self::Result {
        // NOTE: we are not iterating over the subs at this time. Id is just inefficient.Iterator
        // when a message is sent out on the channel we will cleanup the subs with dead Ids
        self.sessions.remove(&msg.0);
    }
}

// server sends this messages to session
#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct BroadcastText(
    pub ChannelId,
    pub crate::SessionId,
    pub Arc<String>,
    pub Dest,
);

// server sends this messages to session
#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct BroadcastBytes(
    pub ChannelId,
    pub crate::SessionId,
    pub Arc<bytes::Bytes>,
    pub Dest,
);

// New session is created
#[derive(Message)]
#[rtype(result = "SubscribeResponce")]
pub struct Subscribe {
    pub channel: String,
    pub addr_text: Recipient<BroadcastText>,
    pub addr_bin: Recipient<BroadcastBytes>,
}

#[derive(MessageResponse)]
pub struct SubscribeResponce {
    pub channel_id: ChannelId,
    pub session_id: SessionId,
}

// Session is disconnected
#[derive(Message)]
#[rtype(result = "()")]
pub struct UnsubscribeAll(pub SessionId);
