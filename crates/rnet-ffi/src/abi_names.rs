//! Public C spelling aliases. Layouts remain defined by the original Rust FFI types.

/// Length-delimited borrowed bytes; strings use UTF-8 without a required terminator.
pub type RnetSliceT = crate::abi::RnetSlice;

/// Optional bounded asynchronous log sink. Descriptor is copied; context is not owned.
pub type RnetLoggerT = crate::abi::RnetLogger;

/// Runtime resource/security settings; initialize with rnet_config_init, then customize.
/// Copied at creation; editing this struct afterwards does not reconfigure a runtime.
pub type RnetConfigT = crate::abi_config::RnetConfig;

/// Client identity and server trust for all outbound connections from a runtime.
pub type RnetClientSecurityT = crate::abi::RnetClientSecurity;

/// Authenticated listener configuration; transport is fixed for its lifetime.
pub type RnetServerConfigT = crate::abi::RnetServerConfig;

/// Outbound endpoint configuration; identity/trust belong to the runtime.
pub type RnetClientConfigT = crate::abi::RnetClientConfig;

/// Generated identity; contains secret material that the caller must erase.
pub type RnetKeypairT = crate::abi::RnetKeypair;

/// Optional transport trace metadata; zero-initialize then set the exact ABI header.
pub type RnetSendOptionsT = crate::abi::RnetSendOptions;

/// Transport event. Release every nonzero token, including diagnostic/auth payloads.
pub type RnetEventT = crate::abi::RnetEvent;

/// Runtime-wide counters/gauges; concurrent fields are not a globally atomic snapshot.
pub type RnetMetricsT = crate::abi::RnetMetrics;

/// Approximate histogram percentiles; microseconds, with sample_count indicating availability.
pub type RnetLatencyMetricT = crate::abi::RnetLatencyMetric;

/// Game runtime options. Initialize first; zero fallback semantics are field-specific.
pub type RnetGameConfigT = crate::game_config_abi::RnetGameConfig;

/// Listener policy and inclusive business-version range; copied by listen.
pub type RnetGameServerConfigT = crate::game_config_abi::RnetGameServerConfig;

/// Outbound game join metadata. Connect returns an endpoint, not a ready session.
pub type RnetGameClientConfigT = crate::game_config_abi::RnetGameClientConfig;

/// Game event; fields are meaningful only for the indicated event kind. Both tokens are owned.
pub type RnetGameEventT = crate::game_abi::RnetGameEvent;

/// Game metadata/scheduling. Priority must be explicit; zero is not Normal.
pub type RnetGameSendOptionsT = crate::game_abi::RnetGameSendOptions;

/// Heartbeat quality with transport-specific loss evidence, not universal packet loss.
pub type RnetGameQualityT = crate::game_abi::RnetGameQuality;

/// Approximate client-side monotonic clock sample; not UTC or an anti-cheat time authority.
pub type RnetGameClockSyncT = crate::game_abi::RnetGameClockSync;

/// Cumulative game telemetry and current gauges; independent counters can race a snapshot.
pub type RnetGameMetricsT = crate::game_abi::RnetGameMetrics;

/// Game staging gauges and cumulative LatestOnly admission/forwarding/drop counters.
pub type RnetGameRealtimeQueueT = crate::game_queue_abi::RnetGameRealtimeQueue;

/// Priority-queue gauges/counters; expiry and delay apply only before transport admission.
pub type RnetGameScheduledQueueT = crate::game_queue_abi::RnetGameScheduledQueue;

/// Early-data buffering during business-version negotiation, scoped to this runtime.
pub type RnetGameRangeBufferT = crate::game_range_abi::RnetGameRangeBuffer;

/// TCP/KCP pending-slot statistics; worker pickup ends replacement eligibility.
pub type RnetGameTransportLatestT = crate::game_abi::RnetGameTransportLatest;

/// Owned export buffer; caller releases its token instead of freeing data.
pub type RnetGameBufferT = crate::game_abi::RnetGameBuffer;

/// Opaque owning-runtime handle; zero is invalid.
pub type RnetRuntime = u64;

/// Runtime-local listener/outbound-connection handle, not a send destination.
pub type RnetEndpoint = u64;

/// Runtime-local peer handle used for send/auth/security operations.
pub type RnetSession = u64;
