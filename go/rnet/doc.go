// Package rnet provides a bounded event-driven client/server network runtime.
//
// Listen and Connect select TCP, UDP, or KCP through configuration. Application Send calls use a
// session only; encryption negotiation, decryption, and server-led security changes are automatic.
package rnet
