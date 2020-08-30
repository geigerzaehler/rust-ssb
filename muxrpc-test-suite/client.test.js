const net = require("net");
const assert = require("assert");
const lodash = require("lodash");
const pull = require("pull-stream");
const toPull = require("stream-to-pull-stream");
const muxrpc = require("muxrpc");

const api = require("./api");

suite("client", function () {
  this.timeout(100);

  setup(async function () {
    this.client = muxrpc(api.manifest, null)();
    const clientStream = this.client.createStream();

    let serverStream;
    if (process.env.EXTERNAL_SERVER) {
      serverStream = toPull.duplex(net.connect(process.env.EXTERNAL_SERVER));
    } else {
      this.server = muxrpc(null, api.manifest)(api.funcs);
      serverStream = this.server.createStream();
    }

    pull(serverStream, clientStream, serverStream);
  });

  teardown(async function () {
    if (this.client || !process.env.EXTERNAL_SERVER) {
      await close(this.client);
    }
    if (this.server) {
      await close(this.client);
    }

    function close(endpoint) {
      return new Promise((resolve, reject) => {
        endpoint.close((err) => {
          if (err && err !== true) {
            reject(err);
          } else {
            resolve();
          }
        });
      });
    }
  });

  test("echo", async function () {
    const response = await this.client.echo("foo");
    assert.equal(response, "foo");
  });

  test("echoSource", async function () {
    const values = [1, 2, 3, 4, 5, 6];
    const valuesResult = await collect(this.client.echoSource(values));
    assert.deepStrictEqual(valuesResult, values);
  });

  test("errorAsync", async function () {
    const error = { name: "NAME", message: "MSG" };
    const errorResult = await this.client.errorAsync(error).catch((e) => e);
    assert.deepStrictEqual(errorResult, error);
  });

  test("add (no-rust)", async function () {
    const values = [1, 2, 3, 4, 5, 6];
    const { sink, source } = this.client.add(2);
    const added = values.map((x) => x + 2);
    pull(pull.values(values), sink);
    const addedResult = await collect(source);
    assert.deepStrictEqual(addedResult, added);
  });

  test.skip("take (no-rust)", async function () {
    const values = [1, 2, 3, 4, 5, 6];
    const take = this.client.take(1);
    const endNotify = throughEndNotify();
    endNotify.ended.then(
      () => {
        console.log("ended");
      },
      () => {
        console.log("ended error");
      }
    );
    pull(pull.values(values), take.sink);
    const outValues = await collect(pull.values(values), endNotify, take);
    assert.deepStrictEqual(outValues, lodash.take(values, 4));
  });

  test("sinkExpect ok (no-rust)", async function () {
    const values = [1, 2, 3, 4, 5, 6];
    await new Promise((resolve, reject) => {
      const sink = this.client.sinkExpect(values, (err) => {
        if (err) {
          reject(err);
        } else {
          resolve();
        }
      });
      pull(pull.values(values), sink);
    });
  });

  test("sinkExpect fail (no-rust)", async function () {
    const values = [1, 2, 3, 4, 5, 6];
    await new Promise((resolve, reject) => {
      const sink = this.client.sinkExpect(values, (err) => {
        if (err) {
          assert.deepEqual(err, {
            name: "not equal",
          });
          resolve();
        } else {
          reject(new Error("Expected error"));
        }
      });
      pull(pull.values(values.slice(0, -1)), sink);
    });
  });

  test("drainAbort (no-rust)", async function () {
    const drain = this.client.drainAbort(1);
    const endNotify = throughEndNotify();
    pull(pullInfiniteThrottled(), endNotify, drain);
    await endNotify.ended;
  });

  test("drainAbortError (no-rust)", async function () {
    const error = { name: "NAME", message: "MSG" };
    const errorResult = await new Promise((resolve, reject) => {
      const drainAbortError = this.client.drainAbortError(4, error, (err) => {
        if (err) {
          resolve(err);
        } else {
          reject(new Error("expected error"));
        }
      });
      pull(pullInfiniteThrottled(), drainAbortError);
    });
    assert.deepEqual(errorResult, error);
  });
});

function pullInfiniteThrottled() {
  return function pullInfiniteThrottledSource(end, cb) {
    setTimeout(() => cb(null, 0), 1);
  };
}

function throughEndNotify() {
  const endDeferred = deferred();
  function through(source) {
    return function cancelNotifySource(end, cb) {
      if (end === true) {
        endDeferred.resolve();
      } else if (end) {
        endDeferred.reject(end);
      }
      source(end, cb);
    };
  }
  through.ended = endDeferred.promise;
  return through;
}

function deferred() {
  const deferred = {};
  deferred.promise = new Promise((resolve, reject) => {
    deferred.resolve = resolve;
    deferred.reject = reject;
  });
  return deferred;
}

function collect(...source) {
  return new Promise((resolve, reject) => {
    pull(
      ...source,
      pull.collect((err, values) => {
        if (err) {
          reject(err);
        } else {
          resolve(values);
        }
      })
    );
  });
}
