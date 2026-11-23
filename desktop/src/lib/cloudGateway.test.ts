import assert from "node:assert/strict";
import {
  canEnableMacGateway,
  classifyGatewayErrorCode,
  gatewayEffectiveBannerKey,
  isValidGatewayUrlInput,
  normalizeGatewayUrlInput,
} from "./cloudGateway";

assert.equal(
  normalizeGatewayUrlInput(" https://gateway.example.com/// "),
  "https://gateway.example.com",
);
assert.equal(isValidGatewayUrlInput(""), true);
assert.equal(isValidGatewayUrlInput("https://gateway.example.com"), true);
assert.equal(isValidGatewayUrlInput("http://127.0.0.1:8787"), true);
assert.equal(isValidGatewayUrlInput("ftp://gateway.example.com"), false);
assert.equal(isValidGatewayUrlInput("https://tok@gateway.example.com"), false);
assert.equal(
  isValidGatewayUrlInput("https://gateway.example.com?token=abc"),
  false,
);

assert.equal(
  gatewayEffectiveBannerKey({
    recognition_mode: "cloud_first",
    gateway_ready: true,
    effective_mode: "cloud_first",
  }),
  "cloudActive",
);
assert.equal(
  gatewayEffectiveBannerKey({
    recognition_mode: "cloud_first",
    gateway_ready: false,
    effective_mode: "local_only",
  }),
  "cloudUnconfiguredLocal",
);
assert.equal(
  gatewayEffectiveBannerKey({
    recognition_mode: "local_only",
    gateway_ready: true,
    effective_mode: "local_only",
  }),
  "localOnly",
);

assert.equal(classifyGatewayErrorCode("gateway_timeout"), "gateway_timeout");
assert.equal(
  classifyGatewayErrorCode("gateway_unauthorized"),
  "gateway_unauthorized",
);
assert.equal(classifyGatewayErrorCode("unexpected_internal"), "generic");
assert.equal(classifyGatewayErrorCode(new Error("boom")), "generic");

const example = "https://gateway.example.com";
const draft = { gateway_url: example, token_configured: true };
assert.equal(canEnableMacGateway(null, "", ""), false);
assert.equal(
  canEnableMacGateway({ ...draft, token_configured: false }, "", example),
  false,
);
assert.equal(canEnableMacGateway(draft, "", ""), false);
assert.equal(canEnableMacGateway(draft, "", example), true);
assert.equal(
  canEnableMacGateway({ ...draft, gateway_url: example + "/" }, example, ""),
  true,
);
assert.equal(
  canEnableMacGateway({ ...draft, gateway_url: example + "/new" }, "", example),
  false,
);

console.log(
  "cloudGateway: URL validation, effective mode, Mac activation, and error mapping assertions passed",
);
