// SCENARIO (654(a) guest==host query ContractId): the query-selfid handler answers 200 with the GUEST-folded
// descriptor().id of the temp.celsius QUERY contract (the temp-query lib, self-reflected via
// contract-descriptor-query) as the raw response body. This asserts that body equals the EXACT 33-byte host
// golden — 0x07 (ContractQuery tag) ++ blake3 of the canonical (kind query)-marked declaration — proving the
// Cadenza guest's query-id fold is byte-identical to the host contract_id_with_kind(_, Query). The same id is
// base62-pinned as 06vdvYIj4KNUp2NXrH17VfKVqXNaI5K5CmsLCpFyexjg0 and cross-pinned host-side in
// src/contract_query_golden.rs + cdz-contract #8913 — so guest fold, wire rule, and construction path all fail
// loudly together on any drift. Drives the handler DIRECTLY as the root program (isolation, no router).
{
  config = {
    root-router = "query-selfid",
    programs = [ { name = "query-selfid", program = "query-selfid" } ],
  },
  requests = [
    // GET / → 200, body = the exact 33-byte query ContractId (0x07 ++ blake3(declaration)) the guest folded.
    { http = { method = "GET", path = "/" },
      expect = { status = 200,
                 body = b"\x07\x13\xd8\x0d\x12\x49\x68\x9a\x06\x2c\x3e\xbd\xbd\x5f\x81\xd2\xbb\xda\x8f\xe7\x8c\x90\x3d\xc9\xd0\x03\x74\xd9\x8a\x60\x3c\x2c\x48" } },
  ],
}
