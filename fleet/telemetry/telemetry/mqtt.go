// SPDX-License-Identifier: MIT OR Apache-2.0

package telemetry

import (
	"errors"
	"fmt"

	"github.com/yosssi/gmq/mqtt/client"

	"tpt-agripod-wire"
)

// SourceConfig says where to connect and how to name this subscriber.
type SourceConfig struct {
	// Address is the broker, host:port. TLS and auth are Phase 3 work (docs/mqtt-topics.md).
	Address  string
	ClientID string
	// KeepAlive is in seconds.
	KeepAlive uint16
	// OnError receives transport errors and handler errors. It may be nil.
	OnError func(error)
}

// Source is a connected MQTT subscription that feeds an Ingester.
type Source struct {
	cli *client.Client
}

// StartSource connects to the broker and subscribes to every node's telemetry at QoS 1.
//
// The client is github.com/yosssi/gmq (MIT). It is the fleet's only MQTT client dependency, so
// replacing it touches this file alone.
func StartSource(cfg SourceConfig, in *Ingester) (*Source, error) {
	if cfg.Address == "" || cfg.ClientID == "" {
		return nil, errors.New("telemetry: source needs an address and a client id")
	}
	onErr := func(err error) {
		if cfg.OnError != nil {
			cfg.OnError(err)
		}
	}
	cli := client.New(&client.Options{ErrorHandler: client.ErrorHandler(onErr)})
	if err := cli.Connect(&client.ConnectOptions{
		Network:      "tcp",
		Address:      cfg.Address,
		ClientID:     []byte(cfg.ClientID),
		CleanSession: true,
		KeepAlive:    cfg.KeepAlive,
	}); err != nil {
		cli.Terminate()
		return nil, fmt.Errorf("telemetry: connect %s: %w", cfg.Address, err)
	}
	handler := func(topic, msg []byte) {
		if err := in.Handle(string(topic), msg); err != nil {
			onErr(err)
		}
	}
	if err := cli.Subscribe(&client.SubscribeOptions{
		SubReqs: []*client.SubReq{{
			TopicFilter: []byte(wire.SubscribeTelemetry),
			QoS:         1,
			Handler:     client.MessageHandler(handler),
		}},
	}); err != nil {
		_ = cli.Disconnect()
		cli.Terminate()
		return nil, fmt.Errorf("telemetry: subscribe: %w", err)
	}
	return &Source{cli: cli}, nil
}

// Close disconnects from the broker.
func (s *Source) Close() error {
	err := s.cli.Disconnect()
	s.cli.Terminate()
	return err
}
