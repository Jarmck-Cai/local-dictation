import assert from "node:assert/strict";
import {
  classifyGatewayErrorCode,
  gatewayEffectiveBannerKey,
  isValidGatewayUrlInput,
  normalizeGatewayUrlInput,
} from "./cloudGateway";

assert.equal(
  normalizeGatewayUrlInput(" https://dictation.astrmind.com/// "),
  "https://dictation.astrmind.com",
);
assert.equal(isValidGatewayUrlInput(""), true);
assert.equal(isValidGatewayUrlInput("https://dictation.astrmind.com"), true);
assert.equal(isValidGatewayUrlInput("http://127.0.0.1:8787"), true);
assert.equal(isValidGatewayUrlInput("ftp://dictation.astrmind.com"), false);
assert.equal(
  isValidGatewayUrlInput("https://tok@dictation.astrmind.com"),
  false,
);
assert.equal(
  isValidGatewayUrlInput("https://dictation.astrmind.com?token=abc"),
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

console.log(
  "cloudGateway: URL validation, effective mode banners, and error mapping assertions passed",
);
