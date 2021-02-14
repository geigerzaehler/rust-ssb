module.exports = {
  env: {
    node: true,
    es2020: true,
  },
  extends: ["eslint:recommended", "plugin:mocha/recommended"],
  overrides: [
    {
      files: ["*.test.js"],
      env: {
        mocha: true,
      },
    },
  ],
  parserOptions: {
    ecmaVersion: 12,
  },
  rules: {
    "mocha/no-skipped-tests": "off",
    "no-unused-vars": [
      "error",
      { vars: "all", args: "after-used", ignoreRestSiblings: false },
    ],
  },
  plugins: ["mocha"],
};
