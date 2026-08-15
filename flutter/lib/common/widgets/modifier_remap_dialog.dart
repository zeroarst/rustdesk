import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/consts.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:get/get.dart';

const String kOptionModifierRemap = 'modifier-remap';

/// Canonical slot names. Must match `Slot::name()` in src/modifier_remap.rs.
const List<String> kModifierSlots = ['ctrl', 'meta', 'alt', 'shift'];

/// "Mac OS" -> "macos". Must match the OS keys the Rust side looks up.
String peerOsKey(String peerPlatform) =>
    peerPlatform.replaceAll(RegExp(r'\s+'), '').toLowerCase();

/// The four slots are neutral in storage but named differently on each OS, so
/// the user never has to translate in their head.
String modifierLabel(String slot, String platform) {
  switch (slot) {
    case 'ctrl':
      return platform == kPeerPlatformMacOS ? 'Control' : 'Ctrl';
    case 'meta':
      if (platform == kPeerPlatformMacOS) return 'Command';
      if (platform == kPeerPlatformWindows) return 'Win';
      return 'Super';
    case 'alt':
      return platform == kPeerPlatformMacOS ? 'Option' : 'Alt';
    case 'shift':
      return 'Shift';
  }
  return slot;
}

String _localPlatformName() {
  if (isMacOS) return kPeerPlatformMacOS;
  if (isWindows) return kPeerPlatformWindows;
  return kPeerPlatformLinux;
}

Map<String, String> loadModifierRemap(String osKey) {
  final raw = bind.mainGetLocalOption(key: kOptionModifierRemap);
  if (raw.isEmpty) return {};
  try {
    final root = jsonDecode(raw);
    if (root is! Map) return {};
    final table = root[osKey];
    if (table is! Map) return {};
    final out = <String, String>{};
    table.forEach((k, v) {
      if (v is String && kModifierSlots.contains(v)) {
        out[k.toString()] = v;
      }
    });
    return out;
  } catch (_) {
    // A malformed option must never block the dialog; fall back to no remap.
    return {};
  }
}

Future<void> saveModifierRemap(String osKey, Map<String, String> table) async {
  final raw = bind.mainGetLocalOption(key: kOptionModifierRemap);
  Map<String, dynamic> root = {};
  if (raw.isNotEmpty) {
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map) root = Map<String, dynamic>.from(decoded);
    } catch (_) {}
  }
  final isIdentity = kModifierSlots.every((s) => (table[s] ?? s) == s);
  if (isIdentity) {
    root.remove(osKey);
  } else {
    root[osKey] = {for (final s in kModifierSlots) s: table[s] ?? s};
  }
  await bind.mainSetLocalOption(
      key: kOptionModifierRemap, value: root.isEmpty ? '' : jsonEncode(root));
}

void showModifierRemapDialog(
    String peerPlatform, OverlayDialogManager dialogManager) {
  final osKey = peerOsKey(peerPlatform);
  final saved = loadModifierRemap(osKey);
  final table = <String, String>{
    for (final s in kModifierSlots) s: saved[s] ?? s
  };
  final localPlatform = _localPlatformName();
  final isMacTarget = peerPlatform == kPeerPlatformMacOS;

  dialogManager.show((setState, close, context) {
    void applyPreset(Map<String, String> preset) => setState(() {
          for (final s in kModifierSlots) {
            table[s] = preset[s] ?? s;
          }
        });

    return CustomAlertDialog(
      title: Text(translate('Modifier keys')),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('${translate('When controlling')}: $peerPlatform')
              .marginOnly(bottom: 12),
          ...kModifierSlots.map((from) => Row(
                children: [
                  SizedBox(
                      width: 96,
                      child: Text(modifierLabel(from, localPlatform))),
                  const Icon(Icons.arrow_forward, size: 16)
                      .marginSymmetric(horizontal: 8),
                  Expanded(
                    child: DropdownButton<String>(
                      isExpanded: true,
                      value: table[from],
                      onChanged: (v) {
                        if (v != null) setState(() => table[from] = v);
                      },
                      items: kModifierSlots
                          .map((to) => DropdownMenuItem(
                              value: to,
                              child: Text(modifierLabel(to, peerPlatform))))
                          .toList(),
                    ),
                  ),
                ],
              ).marginOnly(bottom: 4)),
          const Divider(),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              // `dialogButton` translates its label itself, so pass raw keys.
              dialogButton('No remap',
                  isOutline: true, onPressed: () => applyPreset({})),
              dialogButton('Swap Ctrl/Cmd',
                  isOutline: true,
                  onPressed: () =>
                      applyPreset({'ctrl': 'meta', 'meta': 'ctrl'})),
              if (isMacTarget)
                dialogButton('Mac positional',
                    isOutline: true,
                    onPressed: () => applyPreset(
                        {'ctrl': 'ctrl', 'meta': 'alt', 'alt': 'meta'})),
            ],
          ),
        ],
      ),
      actions: [
        dialogButton('Cancel', onPressed: close, isOutline: true),
        dialogButton('OK', onPressed: () async {
          await saveModifierRemap(osKey, table);
          close();
        }),
      ],
      onCancel: close,
    );
  });
}
