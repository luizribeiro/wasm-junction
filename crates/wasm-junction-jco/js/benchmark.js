// @ts-check

/** @typedef {{ channel: MessageChannel, finish: ((gap: number) => void) | null, last: number, longest: number, remaining: number }} Heartbeat */

/** @returns {number} */
export function benchmarkNow() {
  return performance.now();
}

/** @param {string} message */
export function benchmarkReport(message) {
  console.log(message);
}

/** @returns {Heartbeat} */
export function benchmarkHeartbeatStart() {
  const channel = new MessageChannel();
  /** @type {Heartbeat} */
  const heartbeat = {
    channel,
    finish: null,
    last: performance.now(),
    // The longest gap accumulates from start, so a blocked thread records the reload stall.
    longest: 0,
    remaining: 0,
  };
  channel.port1.onmessage = () => {
    const now = performance.now();
    heartbeat.longest = Math.max(heartbeat.longest, now - heartbeat.last);
    heartbeat.last = now;
    heartbeat.remaining -= 1;
    if (heartbeat.finish && heartbeat.remaining === 0) {
      heartbeat.channel.port1.close();
      heartbeat.channel.port2.close();
      heartbeat.finish(heartbeat.longest);
    } else {
      heartbeat.channel.port2.postMessage(undefined);
    }
  };
  channel.port2.postMessage(undefined);
  return heartbeat;
}

/**
 * @param {Heartbeat} heartbeat
 * @param {number} beats
 * @returns {Promise<number>}
 */
export function benchmarkHeartbeatFinish(heartbeat, beats) {
  return new Promise(resolve => {
    heartbeat.finish = resolve;
    heartbeat.remaining = beats;
  });
}
