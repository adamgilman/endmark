# Relay

Relay forwards webhook events from the public edge to internal consumers.
It listens on port 8080 by default and keeps no state of its own beyond a
delivery queue on local disk.

## Delivery

Each event is written to the queue before Relay answers the sender, so an
accepted event survives a restart. Each consumer acknowledges an event
within 30 seconds or the delivery counts as failed. Handlers must be
idempotent, because an event can arrive more than once.

Events for one consumer are delivered in the order they were accepted.
Events for different consumers are independent of each other.

See the runbook for details.

## Retries

Failed deliveries are retried 3 times with a fixed 5 second delay. After
the last attempt the event moves to the dead-letter queue, where it stays
for 14 days.

```yaml
retries:
  max_attempts: 3
  delay: 5s
  dead_letter_days: 14
```

Operators can replay the dead-letter queue by hand.

See the runbook for details.

## Security

If `verify_signature` is on, Relay rejects any event whose signature does
not match the shared secret, and answers 401 without queueing it.

If `verify_timestamp` is on, Relay rejects any event whose timestamp is
more than five minutes old, and answers 401 without queueing it.

- Rotate the signing secret every 90 days.
- Never log the body of a rejected event.
- Keep the admin port closed to the public network.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `RELAY_PORT` | 8080 | The port Relay listens on |
| `RELAY_TIMEOUT` | 30s | How long to wait for a consumer |
| `RELAY_QUEUE_DIR` | /var/lib/relay | Where the delivery queue lives |

Every variable can also be set in `relay.toml`, and the environment wins
when both are set.
