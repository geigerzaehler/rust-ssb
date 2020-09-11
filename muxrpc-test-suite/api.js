const { callbackify } = require("util");
const lodash = require("lodash");
const pull = require("pull-stream");

const api = {
  // Responds with the first argument
  asyncEcho: {
    type: "async",
    func: async (value) => value,
  },

  // Always responds with an error. Uses the first argument as the
  // error object.
  asyncError: {
    type: "async",
    func: async (error) => {
      throw error;
    },
  },

  // Takes an array of values as the first argument and streams the
  // values back.
  sourceEcho: {
    type: "source",
    func: (values) => {
      return pull.values(values);
    },
  },

  // Emits an error on the source after emitting the given values.
  sourceError: {
    type: "source",
    func: (values, error) => {
      values = values.slice();
      return function (_end, cb) {
        if (values.length == 0) {
          cb(error);
        } else {
          cb(null, values.pop());
        }
      };
    },
  },

  // Source that emits the value `0` every millisecond.
  sourceInifite: {
    type: "source",
    func: () => {
      return function pullInfiniteThrottledSource(end, cb) {
        if (!end) {
          setTimeout(() => cb(null, 0), 1);
        }
      };
    },
  },

  // Takes a number as an argument and a stream of numbers. Streams the
  // numbers back adding the number argument.
  add: {
    type: "duplex",
    func: (value) => {
      return pull.map((x) => x + value);
    },
  },

  // Takes a number `n` as an argument. Consumes `n` items of the input
  // stream and sends them back on the output stream. Then stops the
  // output stream.
  take: {
    type: "duplex",
    func: (n) => {
      return pull.take(n);
    },
  },

  // Takes an array of values as the first argument. Consumes the input
  // stream and expects the stream values to be the input array. If not
  // returns an error.
  sinkExpect: {
    type: "sink",
    func: (values) => {
      const collectedValues = [];
      const sink = pull.drain(
        (value) => {
          collectedValues.push(value);
        },
        () => {
          if (!lodash.isEqual(collectedValues, values)) {
            sink.abort({
              name: "not equal",
            });
          }
        },
      );
      return sink;
    },
  },

  // Takes a number `n` as an argument. Consumes `n` items from the
  // input stream and then aborts.
  sinkAbort: {
    type: "sink",
    func: (n) => {
      return pull(pull.take(n), pull.drain());
    },
  },

  // Takes a number `n` and an error `e` as arguments. Consumes `n`
  // items from the input stream and then aborts with error `e`
  sinkAbortError: {
    type: "sink",
    func: (n, err) => {
      const sink = pull.drain(() => {
        n -= 1;
        if (n === 0) {
          sink.abort(err);
          return false;
        }
      });
      return sink;
    },
  },
};

module.exports.api = api;
module.exports.manifest = lodash.mapValues(api, (x) => x.type);
module.exports.funcs = lodash.mapValues(api, ({ type, func }) => {
  if (type === "async") {
    return callbackify(func);
  } else {
    return func;
  }
});
