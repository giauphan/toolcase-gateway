#[path = "gateway/common.rs"]
mod common;

#[path = "gateway/core_http_tests.rs"]
mod core_http_tests;

#[path = "gateway/core_proxy_tests.rs"]
mod core_proxy_tests;

#[path = "gateway/core_rate_limit_tests.rs"]
mod core_rate_limit_tests;

#[path = "gateway/protocol_omniroute_tests.rs"]
mod protocol_omniroute_tests;

#[path = "gateway/protocol_muse_tests.rs"]
mod protocol_muse_tests;

#[path = "gateway/omniroute_routing_tests.rs"]
mod omniroute_routing_tests;

#[path = "gateway/omniroute_session_tests.rs"]
mod omniroute_session_tests;

#[path = "gateway/muse_video_route_tests.rs"]
mod muse_video_route_tests;

#[path = "gateway/muse_video_generation_tests.rs"]
mod muse_video_generation_tests;

#[path = "gateway/muse_video_polling_tests.rs"]
mod muse_video_polling_tests;

#[path = "gateway/muse_image_route_tests.rs"]
mod muse_image_route_tests;

#[path = "gateway/har_config_tests.rs"]
mod har_config_tests;

#[path = "gateway/har_extraction_tests.rs"]
mod har_extraction_tests;

#[path = "gateway/museai_live_tests.rs"]
mod museai_live_tests;

#[path = "gateway/museai_live_chat_tests.rs"]
mod museai_live_chat_tests;

#[path = "gateway/museai_live_video_tests.rs"]
mod museai_live_video_tests;
