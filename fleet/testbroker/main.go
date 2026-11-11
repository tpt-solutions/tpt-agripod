// SPDX-License-Identifier: MIT OR Apache-2.0

// Command tpt-agripod-testbroker runs an MQTT broker for integration tests. It is not a production
// broker: it allows every client and keeps everything in memory.
//
//	go run ./fleet/testbroker -addr 127.0.0.1:1883
//
// The Rust edge tests read the address from AGRIPOD_TEST_BROKER.
package main

import (
	"flag"
	"fmt"
	"os"
	"os/signal"
	"syscall"

	mqtt "github.com/mochi-mqtt/server/v2"
	"github.com/mochi-mqtt/server/v2/hooks/auth"
	"github.com/mochi-mqtt/server/v2/listeners"
)

func main() {
	addr := flag.String("addr", "127.0.0.1:1883", "listen address")
	flag.Parse()

	server := mqtt.New(&mqtt.Options{})
	if err := server.AddHook(new(auth.AllowHook), nil); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := server.AddListener(listeners.NewTCP(listeners.Config{ID: "test", Address: *addr})); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	go func() {
		if err := server.Serve(); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
	}()
	fmt.Fprintf(os.Stderr, "test broker listening on %s\n", *addr)

	stop := make(chan os.Signal, 1)
	signal.Notify(stop, os.Interrupt, syscall.SIGTERM)
	<-stop
	_ = server.Close()
}
