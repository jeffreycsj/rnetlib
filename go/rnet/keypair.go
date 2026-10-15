package rnet

/*
#include "native.h"
*/
import "C"

import (
	"runtime"
	"unsafe"
)

// Keypair contains caller-owned X25519 identity bytes. Value copies duplicate the secret.
// It is not automatically zeroized and has no redacted formatter: never log this value.
// Erase all private-key copies when no longer needed.
type Keypair struct {
	// Private 32 secret bytes. Value copies duplicate the secret; never log the struct and erase every owned copy.
	Private [32]byte
	// Public corresponding 32-byte public key for distributing trusted pins.
	Public [32]byte
}

// GenerateKeypair creates a fresh identity using the native cryptographic RNG.
// The returned private bytes belong to the caller; generation failures return an error.
func GenerateKeypair() (Keypair, error) {
	if err := checkABI(); err != nil {
		return Keypair{}, err
	}
	defer lockNativeThread()()
	var keypair Keypair
	err := statusError(C.rnet_go_keypair_generate(
		(*C.uint8_t)(unsafe.Pointer(&keypair.Private[0])),
		(*C.uint8_t)(unsafe.Pointer(&keypair.Public[0])),
	))
	return keypair, err
}

// KeypairFromPrivate derives the public key from exactly 32 private bytes.
// The input is copied, not erased; the caller owns both original and returned secrets.
func KeypairFromPrivate(private []byte) (Keypair, error) {
	if err := checkABI(); err != nil {
		return Keypair{}, err
	}
	defer lockNativeThread()()
	var keypair Keypair
	if len(private) != len(keypair.Private) {
		return keypair, StatusError{Code: int32(C.RNET_E_INVALID_ARGUMENT), Message: "private key must contain exactly 32 bytes"}
	}
	copy(keypair.Private[:], private)
	err := statusError(C.rnet_go_keypair_from_private(
		bytePointer(private), C.size_t(len(private)),
		(*C.uint8_t)(unsafe.Pointer(&keypair.Public[0])),
	))
	runtime.KeepAlive(private)
	return keypair, err
}
