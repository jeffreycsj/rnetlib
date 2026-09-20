#ifndef RNET_KEYPAIR_HPP
#define RNET_KEYPAIR_HPP

#include "rnet/types.hpp"

namespace rnet {

class Keypair {
 public:
  Keypair() {
    raw_.struct_size = sizeof(raw_);
    raw_.abi_version = RNET_ABI_VERSION;
    check(rnet_keypair_generate(&raw_));
  }

  static Keypair from_private_key(const uint8_t *private_key,
                                  size_t private_key_size = 32) {
    return Keypair(private_key, private_key_size, RestoreTag());
  }

  Keypair(const Keypair &) = delete;
  Keypair &operator=(const Keypair &) = delete;

  Keypair(Keypair &&other) noexcept : raw_(other.raw_) {
    std::fill(other.raw_.private_key, other.raw_.private_key + 32, 0);
    std::fill(other.raw_.public_key, other.raw_.public_key + 32, 0);
  }

  Keypair &operator=(Keypair &&other) noexcept {
    if (this != &other) {
      std::fill(raw_.private_key, raw_.private_key + 32, 0);
      raw_ = other.raw_;
      std::fill(other.raw_.private_key, other.raw_.private_key + 32, 0);
      std::fill(other.raw_.public_key, other.raw_.public_key + 32, 0);
    }
    return *this;
  }

  ~Keypair() {
    volatile uint8_t *bytes =
        reinterpret_cast<volatile uint8_t *>(&raw_.private_key[0]);
    for (size_t index = 0; index < sizeof(raw_.private_key); ++index) {
      bytes[index] = 0;
    }
  }

  const uint8_t *private_key() const { return &raw_.private_key[0]; }
  const uint8_t *public_key() const { return &raw_.public_key[0]; }

 private:
  struct RestoreTag {};

  Keypair(const uint8_t *private_key, size_t private_key_size, RestoreTag) {
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
