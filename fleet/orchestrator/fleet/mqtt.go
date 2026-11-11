// SPDX-License-Identifier: MIT OR Apache-2.0

package fleet

import (
	"errors"
	"fmt"

	"github.com/yosssi/gmq/mqtt/client"
)

// Link is a connected MQTT session the pusher and registry use. The client is
// github.com/yosssi/gmq (MIT). It is the only MQTT client dependency, so swapping it touches this
// file alone (ADR 0007).
type Link struct {
	cli *client.Client
}

// Connect opens a session to the broker at addr. onErr receives transport and handler errors.
func Connect(addr, clientID string, keepAlive uint16, onErr func(error)) (*Link, error) {
	if addr == "" || clientID == "" {
		return nil, errors.New("fleet: link needs an address and a client id")
	}
	if onErr == nil {
		onErr = func(error) {}
	}
	cli := client.New(&client.Options{ErrorHandler: client.ErrorHandler(onErr)})
	if err := cli.Connect(&client.ConnectOptions{
		Network:      "tcp",
		Address:      addr,
		ClientID:     []byte(clientID),
		CleanSession: true,
		KeepAlive:    keepAlive,
	}); err != nil {
		cli.Terminate()
		return nil, fmt.Errorf("fleet: connect %s: %w", addr, err)
	}
	return &Link{cli: cli}, nil
}

// Publish sends payload on topic at QoS 1. It satisfies Publisher.
func (l *Link) Publish(topic string, payload []byte) error {
	return l.cli.Publish(&client.PublishOptions{
		QoS:       1,
		TopicName: []byte(topic),
		Message:   payload,
	})
}

// Subscribe registers h for every message matching filter, at QoS 1.
func (l *Link) Subscribe(filter string, h func(topic, payload []byte)) error {
	return l.cli.Subscribe(&client.SubscribeOptions{
		SubReqs: []*client.SubReq{{
			TopicFilter: []byte(filter),
			QoS:         1,
			Handler:     client.MessageHandler(h),
		}},
	})
}

// Close disconnects cleanly.
func (l *Link) Close() error {
	err := l.cli.Disconnect()
	l.cli.Terminate()
	return err
}
