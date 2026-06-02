/// Hub roster selection policy after reload (see [preserveHubConversationSelection]).
///
/// Regression guard: after scanning contact A, a fast roster reload must not
/// switch the open chat to contact B via `list.first`.
String? preserveHubConversationSelection({
  required String? selectedConversationKey,
  required List<String> rosterKeys,
}) {
  // Intentionally ignore [rosterKeys] for the missing-row case — keep selection
  // until the upserted contact appears on the next reload.
  return selectedConversationKey;
}
