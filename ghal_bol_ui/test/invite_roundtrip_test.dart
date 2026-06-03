import "package:flutter_test/flutter_test.dart";
import "package:ghal_bol_ui/ghalbol_connect_invite.dart";
import "package:ghal_bol_ui/invite_uri_builder.dart";
import "package:ghal_bol_ui/invite_uri_codec.dart";

void main() {
  const pk =
      "02f229f167ac2337144dbeba4392a6300c8fe97fb061efdb4f81ec9f29dec76936";

  test("invite links include alias on web and app URIs", () {
    final links = buildGhalBolInviteLinks(
      publicKeyHex: pk,
      peerAlias: "Mearaj",
    );
    expect(links, isNotNull);
    expect(links!.httpsUri, contains("?alias=Mearaj"));
    expect(links.appUri, "ghalbol://connect/$pk?alias=Mearaj");
  });

  test("HTTPS encode/decode roundtrip", () {
    final wire = {
      "ghalbol.share": "ghal_bol_connect_v1",
      "format_version": 2,
      "topic": "ghal-bol-chat",
      "public_key_hex": pk,
      "peer_alias": "Test",
    };
    final uri = encodeConnectInviteUri(wire);
    expect(
      uri,
      "https://ghalbol.com/connect/$pk?alias=Test",
    );
    final dec = decodeConnectInviteUri(uri);
    expect(dec, isNotNull);
    expect(dec!["public_key_hex"], pk);
    expect(dec["peer_alias"], "Test");
    expect(verifyConnectInviteWireMap(dec), isTrue);
  });

  test("www.ghalbol.com invite decodes", () {
    final https = "https://www.ghalbol.com/connect/$pk?alias=Mearaj";
    expect(decodeConnectInviteUri(https), isNotNull);
    expect(inviteAppUriFromHttps(https), "ghalbol://connect/$pk?alias=Mearaj");
  });

  test("HTTPS location to ghalbol app URI", () {
    final https =
        "https://ghalbol.com/connect/$pk?alias=Probrine99";
    final app = inviteAppUriFromHttps(https);
    expect(app, "ghalbol://connect/$pk?alias=Probrine99");
    final loc = Uri.parse(https);
    expect(inviteAppUriFromUri(loc), app);
    expect(inviteHttpsStringFromUri(loc), https);
  });

  test("app scheme encode/decode roundtrip", () {
    final wire = {
      "ghalbol.share": "ghal_bol_connect_v1",
      "format_version": 2,
      "topic": "ghal-bol-chat",
      "public_key_hex": pk,
    };
    final uri = encodeConnectInviteAppUri(wire);
    expect(uri, "ghalbol://connect/$pk");
    final dec = decodeConnectInviteUri(uri);
    expect(dec, isNotNull);
    expect(dec!["public_key_hex"], pk);
  });

  test("coord query in pasted URI is ignored", () {
    final wire = {
      "ghalbol.share": "ghal_bol_connect_v1",
      "format_version": 2,
      "topic": "ghal-bol-chat",
      "public_key_hex": pk,
    };
    expect(encodeConnectInviteUri(wire), isNot(contains("coord=")));
    final dec = decodeConnectInviteUri(
      "https://ghalbol.com/connect/$pk?coord=http%3A%2F%2F192.168.1.38%3A8765",
    );
    expect(dec, isNotNull);
    expect(dec!.containsKey("coord_base_url"), isFalse);
    expect(verifyConnectInviteWireMap(dec), isTrue);
  });

  test("extractConnectInviteUri preserves alias from QR-style payload", () {
    const withAlias =
        "https://ghalbol.com/connect/$pk?alias=Mearaj";
    final extracted = extractConnectInviteUri(withAlias);
    expect(extracted, withAlias);
    final inv = GhalBolConnectInvite.tryParseInviteUri(extracted!);
    expect(inv?.peerAlias, "Mearaj");
  });

  test("extractConnectInviteUri finds HTTPS in wrapper text", () {
    const wrapped =
        "Join me: https://ghalbol.com/connect/$pk thanks";
    final extracted = extractConnectInviteUri(wrapped);
    expect(extracted, startsWith("https://ghalbol.com/connect/"));
  });
}
