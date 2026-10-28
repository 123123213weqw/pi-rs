// pi extension: hijack deepseek baseUrl to the local mock server.
// Usage: pi -e /path/to/mock_provider.js ...
export default function (pi) {
  pi.registerProvider("deepseek", {
    baseUrl: process.env.MOCK_BASE_URL || "http://127.0.0.1:8999",
  });
}
