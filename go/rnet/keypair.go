package rnet

/*
#include "native.h"
*/
import "C"

import (
	"runtime"
	"unsafe"
)

type Keypair struct {
	Private [32]byte
	Public  [32]byte
}

func GenerateKeypair() (Keypair, error) {
	var keypair Keypair
	err := statusError(C.rnet_go_keypair_generate(
		(*C.uint8_t)(unsafe.Pointer(&keypair.Private[0])),
		(*C.uint8_t)(unsafe.Pointer(&keypair.Public[0])),
	))
	return keypair, err
}

func KeypairFromPrivate(private []byte) (Keypair, error) {
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
