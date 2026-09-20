# Protobuf ownership

RNet does not own business message schemas or a handwritten `msg_type` mapping. Applications keep `.proto` files and generated mapping tables in their protocol repository, then use `rnet_protocol::encode_protobuf` / `decode_protobuf` when Rust-side control-message decoding is desired. Other callers can poll raw frame bodies and decode them in their own language.

Pin `protoc` in the application build image, never reuse field numbers, reserve removed fields, and set a decode limit no larger than the runtime frame body limit.

