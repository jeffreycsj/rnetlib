#ifndef RNET_KEYPAIR_HPP
#define RNET_KEYPAIR_HPP

#include "rnet/types.hpp"

namespace rnet {

/// Move-only 32-byte identity. Destructor erases its private key; caller-made copies are not erased.
/// Do not log private material or use moved-from keys. Public key is safe to distribute for pinning.
class Keypair {
 public:
  /// Generates an identity; may throw Error if native generation/ABI validation fails.
  Keypair() {
    check_abi();
    raw_.struct_size = sizeof(raw_);
    raw_.abi_version = RNET_ABI_VERSION;
    check(rnet_keypair_generate(&raw_));
  }

  /// Copies exactly private_key_size=32 bytes and derives public key; wrong length throws Error.
  /// Null with nonzero size throws invalid_argument. Caller still owns/erases the input secret.
  static Keypair from_private_key(const uint8_t *private_key,
                                  size_t private_key_size = 32) {
    return Keypair(private_key, private_key_size, RestoreTag());
  }

  /// Secrets are not implicitly duplicated.
  Keypair(const Keypair &) = delete;
  /// Secrets are not implicitly duplicated.
  Keypair &operator=(const Keypair &) = delete;

  /// Transfers identity and clears the moved-from arrays; do not use other as an identity.
  Keypair(Keypair &&other) noexcept : raw_(other.raw_) {
    std::fill(other.raw_.private_key, other.raw_.private_key + 32, 0);
    std::fill(other.raw_.public_key, other.raw_.public_key + 32, 0);
  }

  /// Replaces identity, clearing old/moved-from arrays; synchronize against readers.
  Keypair &operator=(Keypair &&other) noexcept {
    if (this != &other) {
      std::fill(raw_.private_key, raw_.private_key + 32, 0);
      raw_ = other.raw_;
      std::fill(other.raw_.private_key, other.raw_.private_key + 32, 0);
      std::fill(other.raw_.public_key, other.raw_.public_key + 32, 0);
    }
    return *this;
  }

  /// Erases this object's private-key array; does not erase copies made by callers.
  ~Keypair() {
    volatile uint8_t *bytes =
        reinterpret_cast<volatile uint8_t *>(&raw_.private_key[0]);
    for (size_t index = 0; index < sizeof(raw_.private_key); ++index) {
      bytes[index] = 0;
    }
  }

  /// Borrowed 32 secret bytes; valid until move/destruction. Never log or retain this pointer.
  const uint8_t *private_key() const { return &raw_.private_key[0]; }
  /// Borrowed 32 public bytes; copy before moving/destroying this object.
  const uint8_t *public_key() const { return &raw_.public_key[0]; }

 private:
  struct RestoreTag {};

  Keypair(const uint8_t *private_key, size_t private_key_size, RestoreTag) {
    check_abi();
    if (private_key == NULL && private_key_size != 0) {
      throw std::invalid_argument("private key is null");
    }
    rnet_slice_t bytes = {private_key, private_key_size};
    check(rnet_keypair_from_private(bytes, &raw_));
  }

  rnet_keypair_t raw_;
};

}  // namespace rnet

#endif
