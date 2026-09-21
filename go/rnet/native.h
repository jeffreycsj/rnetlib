#ifndef RNET_GO_NATIVE_H
#define RNET_GO_NATIVE_H

#include "rnet.h"
#include <string.h>

static inline void rnet_go_log_v2_shim(
    void *user_data, uint64_t timestamp_unix_ms, uint32_t level,
    const uint8_t *event_name, size_t event_name_len, rnet_runtime_t runtime,
    rnet_endpoint_t endpoint, rnet_session_t session, uint32_t transport,
    int32_t error_code, uint64_t correlation_id, const uint8_t *message,
    size_t message_len);

static inline int32_t rnet_go_game_runtime_create(
    const rnet_game_config_t *config, const rnet_config_v5_t *network_config,
    const uint8_t *client_private_key,
    const uint8_t *server_public_key, uintptr_t logger_handle,
    uint32_t min_log_level, rnet_runtime_t *out) {
  rnet_game_config_t configured = *config;
  configured.network_config = network_config;
  rnet_logger_v2_t logger = {0};
  if (logger_handle != 0) {
    logger.struct_size = sizeof(logger);
    logger.abi_version = RNET_ABI_VERSION;
    logger.log = rnet_go_log_v2_shim;
    logger.user_data = (void *)logger_handle;
    logger.min_level = min_log_level;
  }
  if (client_private_key == NULL || server_public_key == NULL) {
    return logger_handle == 0
               ? rnet_game_runtime_create(&configured, NULL, out)
               : rnet_game_runtime_create_logged(&configured, NULL, &logger, out);
  }
  rnet_client_security_t security = {0};
  security.struct_size = sizeof(security);
  security.abi_version = RNET_ABI_VERSION;
  security.local_private_key.ptr = client_private_key;
  security.local_private_key.len = 32;
  security.expected_server_public_key.ptr = server_public_key;
  security.expected_server_public_key.len = 32;
  return logger_handle == 0
             ? rnet_game_runtime_create(&configured, &security, out)
             : rnet_game_runtime_create_logged(&configured, &security, &logger, out);
}

static inline int32_t rnet_go_game_server_listen(
    rnet_runtime_t runtime, uint32_t transport, const uint8_t *host,
    size_t host_len, uint16_t port, const uint8_t *private_key,
    uint32_t initial_encryption, uint64_t protocol_id,
    uint32_t protocol_version, rnet_endpoint_t *out) {
  rnet_game_server_config_t config = {0};
  config.struct_size = sizeof(config);
  config.abi_version = RNET_ABI_VERSION;
  config.transport = transport;
  config.initial_encryption = initial_encryption;
  config.bind_host.ptr = host;
  config.bind_host.len = host_len;
  config.bind_port = port;
  config.local_private_key.ptr = private_key;
  config.local_private_key.len = 32;
  config.protocol_id = protocol_id;
  config.protocol_version = protocol_version;
  return rnet_game_server_listen(runtime, &config, out);
}

static inline int32_t rnet_go_game_client_connect(
    rnet_runtime_t runtime, uint32_t transport, const uint8_t *host,
    size_t host_len, uint16_t port, const uint8_t *join_ticket,
    size_t join_ticket_len, uint64_t protocol_id, uint32_t protocol_version,
    uint64_t build_id, uint64_t capabilities, rnet_endpoint_t *out) {
  rnet_game_client_config_t config = {0};
  config.struct_size = sizeof(config);
  config.abi_version = RNET_ABI_VERSION;
  config.transport = transport;
  config.remote_host.ptr = host;
  config.remote_host.len = host_len;
  config.remote_port = port;
  config.join_ticket.ptr = join_ticket;
  config.join_ticket.len = join_ticket_len;
  config.protocol_id = protocol_id;
  config.protocol_version = protocol_version;
  config.build_id = build_id;
  config.capabilities = capabilities;
  return rnet_game_client_connect(runtime, &config, out);
}

static inline int32_t rnet_go_game_client_resume_connect(
    rnet_runtime_t runtime, uint32_t transport, const uint8_t *host,
    size_t host_len, uint16_t port, const uint8_t *join_ticket,
    size_t join_ticket_len, uint64_t protocol_id, uint32_t protocol_version,
    uint64_t build_id, uint64_t capabilities, rnet_session_t old_session,
    const uint8_t *resume_ticket, size_t resume_ticket_len,
    rnet_endpoint_t *out) {
  rnet_game_client_config_t config = {0};
  config.struct_size = sizeof(config);
  config.abi_version = RNET_ABI_VERSION;
  config.transport = transport;
  config.remote_host.ptr = host;
  config.remote_host.len = host_len;
  config.remote_port = port;
  config.join_ticket.ptr = join_ticket;
  config.join_ticket.len = join_ticket_len;
  config.protocol_id = protocol_id;
  config.protocol_version = protocol_version;
  config.build_id = build_id;
  config.capabilities = capabilities;
  rnet_slice_t ticket = {resume_ticket, resume_ticket_len};
  return rnet_game_client_resume_connect(runtime, &config, old_session,
                                         ticket, out);
}

extern void rnet_go_log_v2_bridge(
    void *user_data, uint64_t timestamp_unix_ms, uint32_t level,
    uint8_t *event_name, size_t event_name_len, rnet_runtime_t runtime,
    rnet_endpoint_t endpoint, rnet_session_t session, uint32_t transport,
    int32_t error_code, uint64_t correlation_id, uint8_t *message,
    size_t message_len);

static inline void rnet_go_log_v2_shim(
    void *user_data, uint64_t timestamp_unix_ms, uint32_t level,
    const uint8_t *event_name, size_t event_name_len, rnet_runtime_t runtime,
    rnet_endpoint_t endpoint, rnet_session_t session, uint32_t transport,
    int32_t error_code, uint64_t correlation_id, const uint8_t *message,
    size_t message_len) {
  rnet_go_log_v2_bridge(
      user_data, timestamp_unix_ms, level, (uint8_t *)event_name,
      event_name_len, runtime, endpoint, session, transport, error_code,
      correlation_id, (uint8_t *)message, message_len);
}

static inline int32_t rnet_go_runtime_create_v2(
    const rnet_config_t *config, const uint8_t *client_private_key,
    const uint8_t *server_public_key, rnet_runtime_t *out) {
  rnet_client_security_t security = {0};
  security.struct_size = sizeof(security);
  security.abi_version = RNET_ABI_VERSION;
  security.local_private_key.ptr = client_private_key;
  security.local_private_key.len = 32;
  security.expected_server_public_key.ptr = server_public_key;
  security.expected_server_public_key.len = 32;
  return rnet_runtime_create_v2(config, &security, out);
}

static inline int32_t rnet_go_runtime_create_v3(
    const rnet_config_v3_t *config, const uint8_t *client_private_key,
    const uint8_t *server_public_key, rnet_runtime_t *out) {
  rnet_client_security_t security = {0};
  security.struct_size = sizeof(security);
  security.abi_version = RNET_ABI_VERSION;
  security.local_private_key.ptr = client_private_key;
  security.local_private_key.len = 32;
  security.expected_server_public_key.ptr = server_public_key;
  security.expected_server_public_key.len = 32;
  return rnet_runtime_create_v3(config, &security, out);
}

static inline int32_t rnet_go_runtime_create_v4(
    const rnet_config_v4_t *config, const uint8_t *client_private_key,
    const uint8_t *server_public_key, rnet_runtime_t *out) {
  rnet_client_security_t security = {0};
  security.struct_size = sizeof(security);
  security.abi_version = RNET_ABI_VERSION;
  security.local_private_key.ptr = client_private_key;
  security.local_private_key.len = 32;
  security.expected_server_public_key.ptr = server_public_key;
  security.expected_server_public_key.len = 32;
  return rnet_runtime_create_v4(config, &security, out);
}

static inline int32_t rnet_go_runtime_create_v4_full(
    const rnet_config_v4_t *config, const uint8_t *client_private_key,
    const uint8_t *server_public_key, uintptr_t logger_handle,
    uint32_t min_log_level, rnet_runtime_t *out) {
  rnet_config_v4_t configured = *config;
  rnet_logger_v2_t logger = {0};
  if (logger_handle != 0) {
    logger.struct_size = sizeof(logger);
    logger.abi_version = RNET_ABI_VERSION;
    logger.log = rnet_go_log_v2_shim;
    logger.user_data = (void *)logger_handle;
    logger.min_level = min_log_level;
    configured.logger_v2 = &logger;
  }
  if (client_private_key == NULL || server_public_key == NULL) {
    return rnet_runtime_create_v4(&configured, NULL, out);
  }
  rnet_client_security_t security = {0};
  security.struct_size = sizeof(security);
  security.abi_version = RNET_ABI_VERSION;
  security.local_private_key.ptr = client_private_key;
  security.local_private_key.len = 32;
  security.expected_server_public_key.ptr = server_public_key;
  security.expected_server_public_key.len = 32;
  return rnet_runtime_create_v4(&configured, &security, out);
}

static inline int32_t rnet_go_runtime_create_v5_full(
    const rnet_config_v5_t *config, const uint8_t *client_private_key,
    const uint8_t *server_public_key, uintptr_t logger_handle,
    uint32_t min_log_level, rnet_runtime_t *out) {
  rnet_config_v5_t configured = *config;
  rnet_logger_v2_t logger = {0};
  if (logger_handle != 0) {
    logger.struct_size = sizeof(logger);
    logger.abi_version = RNET_ABI_VERSION;
    logger.log = rnet_go_log_v2_shim;
    logger.user_data = (void *)logger_handle;
    logger.min_level = min_log_level;
    configured.logger_v2 = &logger;
  }
  if (client_private_key == NULL || server_public_key == NULL) {
    return rnet_runtime_create_v5(&configured, NULL, out);
  }
  rnet_client_security_t security = {0};
  security.struct_size = sizeof(security);
  security.abi_version = RNET_ABI_VERSION;
  security.local_private_key.ptr = client_private_key;
  security.local_private_key.len = 32;
  security.expected_server_public_key.ptr = server_public_key;
  security.expected_server_public_key.len = 32;
  return rnet_runtime_create_v5(&configured, &security, out);
}

static inline int32_t rnet_go_server_open_v2(
    rnet_runtime_t runtime, uint32_t transport, uint32_t initial_security,
    const uint8_t *host, size_t host_len, uint16_t port,
    const uint8_t *private_key, rnet_endpoint_t *out) {
  rnet_server_config_v2_t config = {0};
  config.struct_size = sizeof(config);
  config.abi_version = RNET_ABI_VERSION;
  config.transport = transport;
  config.initial_security = initial_security;
  config.bind_host.ptr = host;
  config.bind_host.len = host_len;
  config.bind_port = port;
  config.local_private_key.ptr = private_key;
  config.local_private_key.len = 32;
  return rnet_server_open_v2(runtime, &config, out);
}

static inline int32_t rnet_go_client_connect_v2(
    rnet_runtime_t runtime, uint32_t transport, const uint8_t *host,
    size_t host_len, uint16_t port, const uint8_t *payload,
    size_t payload_len, rnet_endpoint_t *out) {
  rnet_client_config_v2_t config = {0};
  config.struct_size = sizeof(config);
  config.abi_version = RNET_ABI_VERSION;
  config.transport = transport;
  config.remote_host.ptr = host;
  config.remote_host.len = host_len;
  config.remote_port = port;
  config.join_payload.ptr = payload;
  config.join_payload.len = payload_len;
  return rnet_client_connect_v2(runtime, &config, out);
}

static inline int32_t rnet_go_endpoint_open(
    rnet_runtime_t runtime, uint32_t transport, uint32_t mode,
    const uint8_t *bind_host, size_t bind_host_len, uint16_t bind_port,
    const uint8_t *remote_host, size_t remote_host_len, uint16_t remote_port,
    rnet_endpoint_t *out) {
  rnet_endpoint_config_t config = {0};
  config.struct_size = sizeof(config);
  config.abi_version = RNET_ABI_VERSION;
  config.transport = transport;
  config.mode = mode;
  config.bind_host.ptr = bind_host;
  config.bind_host.len = bind_host_len;
  config.bind_port = bind_port;
  config.remote_host.ptr = remote_host;
  config.remote_host.len = remote_host_len;
  config.remote_port = remote_port;
  return rnet_endpoint_open(runtime, &config, out);
}

static inline int32_t rnet_go_send(
    rnet_runtime_t runtime, rnet_session_t session, uint32_t message_type,
    const uint8_t *payload, size_t payload_len) {
  rnet_slice_t slice = {payload, payload_len};
  return rnet_session_send(runtime, session, message_type, slice);
}

static inline int32_t rnet_go_send_ex(
    rnet_runtime_t runtime, rnet_session_t session, uint32_t message_type,
    const uint8_t *payload, size_t payload_len, uint64_t correlation_id) {
  rnet_slice_t slice = {payload, payload_len};
  rnet_send_options_t options = {0};
  options.struct_size = sizeof(options);
  options.abi_version = RNET_ABI_VERSION;
  options.correlation_id = correlation_id;
  return rnet_session_send_ex(runtime, session, message_type, slice, &options);
}

static inline int32_t rnet_go_send_legacy(
    rnet_runtime_t runtime, rnet_session_t session, uint32_t message_type,
    uint32_t stream_id, const uint8_t *payload, size_t payload_len,
    uint64_t request_id) {
  rnet_slice_t slice = {payload, payload_len};
  return rnet_send(runtime, session, message_type, stream_id, slice,
                   request_id);
}

static inline int32_t rnet_go_keypair_from_private(
    const uint8_t *private_key, size_t private_key_len, uint8_t *public_key) {
  rnet_keypair_t keypair = {0};
  rnet_slice_t private_slice = {private_key, private_key_len};
  int32_t status = rnet_keypair_from_private(private_slice, &keypair);
  if (status == RNET_OK) {
    memcpy(public_key, keypair.public_key, 32);
    memset(keypair.private_key, 0, 32);
  }
  return status;
}

static inline int32_t rnet_go_keypair_generate(uint8_t *private_key,
                                                uint8_t *public_key) {
  rnet_keypair_t keypair = {0};
  keypair.struct_size = sizeof(keypair);
  keypair.abi_version = RNET_ABI_VERSION;
  int32_t status = rnet_keypair_generate(&keypair);
  if (status == RNET_OK) {
    memcpy(private_key, keypair.private_key, 32);
    memcpy(public_key, keypair.public_key, 32);
    memset(keypair.private_key, 0, 32);
  }
  return status;
}

static inline int32_t rnet_go_listener_open(
    rnet_runtime_t runtime, uint32_t transport, const uint8_t *host,
    size_t host_len, uint16_t port, const uint8_t *private_key,
    rnet_endpoint_t *out) {
  rnet_listener_config_t config = {0};
  config.struct_size = sizeof(config);
  config.abi_version = RNET_ABI_VERSION;
  config.transport = transport;
  config.bind_host.ptr = host;
  config.bind_host.len = host_len;
  config.bind_port = port;
  config.local_private_key.ptr = private_key;
  config.local_private_key.len = 32;
  return rnet_listener_open(runtime, &config, out);
}

static inline int32_t rnet_go_client_join(
    rnet_runtime_t runtime, uint32_t transport, const uint8_t *host,
    size_t host_len, uint16_t port, const uint8_t *private_key,
    const uint8_t *server_public_key, const uint8_t *payload,
    size_t payload_len, rnet_endpoint_t *out) {
  rnet_join_config_t config = {0};
  config.struct_size = sizeof(config);
  config.abi_version = RNET_ABI_VERSION;
  config.transport = transport;
  config.remote_host.ptr = host;
  config.remote_host.len = host_len;
  config.remote_port = port;
  config.local_private_key.ptr = private_key;
  config.local_private_key.len = 32;
  config.expected_server_public_key.ptr = server_public_key;
  config.expected_server_public_key.len = 32;
  config.join_payload.ptr = payload;
  config.join_payload.len = payload_len;
  return rnet_client_join(runtime, &config, out);
}

#endif
