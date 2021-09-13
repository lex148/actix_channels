#[derive(Debug)]
pub enum ChannelError {
    StopMessage,
    SendClientText(String),
    SendClientBinary(bytes::Bytes),
}
