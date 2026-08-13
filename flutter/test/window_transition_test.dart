import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_hbb/utils/window_transition.dart';

void main() {
  group('pickAndReserveInactiveWindow', () {
    test('returns null when no window is inactive', () {
      final inactive = <int>{};
      expect(pickAndReserveInactiveWindow([1, 2, 3], inactive), null);
    });

    test('picks the first inactive window in order', () {
      final inactive = {3, 2};
      expect(pickAndReserveInactiveWindow([1, 2, 3], inactive), 2);
    });

    test('reserves synchronously so concurrent picks get distinct windows',
        () {
      // Models the reconnect-after-close-all race: three interleaved
      // openMonitorSession calls must not reuse the same hidden window.
      final inactive = {2, 3, 4};
      final picks = [
        pickAndReserveInactiveWindow([1, 2, 3, 4], inactive),
        pickAndReserveInactiveWindow([1, 2, 3, 4], inactive),
        pickAndReserveInactiveWindow([1, 2, 3, 4], inactive),
      ];
      expect(picks, [2, 3, 4]);
      expect(inactive, isEmpty);
      // Pool exhausted: next caller must create a new window instead.
      expect(pickAndReserveInactiveWindow([1, 2, 3, 4], inactive), null);
    });
  });

  group('WindowTransitionSuppressor', () {
    test('inactive by default', () {
      final s = WindowTransitionSuppressor();
      expect(s.isActive, false);
    });

    test('active during suppression window, expires after', () {
      var now = DateTime(2026, 1, 1);
      final s = WindowTransitionSuppressor(clock: () => now);
      s.suppressFor(const Duration(milliseconds: 1200));
      expect(s.isActive, true);
      now = now.add(const Duration(milliseconds: 1199));
      expect(s.isActive, true);
      now = now.add(const Duration(milliseconds: 2));
      expect(s.isActive, false);
    });

    test('a shorter suppression never shrinks an active longer one', () {
      var now = DateTime(2026, 1, 1);
      final s = WindowTransitionSuppressor(clock: () => now);
      s.suppressFor(const Duration(seconds: 2));
      s.suppressFor(const Duration(milliseconds: 100));
      now = now.add(const Duration(seconds: 1));
      expect(s.isActive, true,
          reason: 'the 2s suppression must still be in force');
    });

    test('a longer suppression extends an active shorter one', () {
      var now = DateTime(2026, 1, 1);
      final s = WindowTransitionSuppressor(clock: () => now);
      s.suppressFor(const Duration(milliseconds: 100));
      s.suppressFor(const Duration(seconds: 2));
      now = now.add(const Duration(seconds: 1));
      expect(s.isActive, true);
    });
  });

  group('per-display local keys', () {
    test('perDisplayLocalKey composes prefix, display and peer', () {
      expect(perDisplayLocalKey('some-pref', 'peer1', 2), 'some-pref-d2_peer1');
    });

    test('taskbarOrderKey format is unchanged by the refactor', () {
      expect(taskbarOrderKey('180824860', 3), 'taskbar-order-d3_180824860');
      expect(taskbarOrderKey('other', 0), 'taskbar-order-d0_other');
    });

    test('toolbar state keys scope to peer and display', () {
      expect(toolbarCollapseKey('180824860', 3),
          'toolbar-collapse-d3_180824860');
      expect(toolbarHideKey('180824860', 0), 'toolbar-hide-d0_180824860');
    });
  });

  group('DisplayLayoutGate', () {
    test('the first live peer info lays the displays out', () {
      expect(DisplayLayoutGate().shouldLayOut(isCache: false), true);
    });

    test('a reconnect does not lay the displays out again', () {
      // The window that laid the session out stays attached to the shared
      // connection, so "Retry" delivers it a second live peer info.
      final gate = DisplayLayoutGate();
      gate.shouldLayOut(isCache: false);
      expect(gate.shouldLayOut(isCache: false), false);
    });

    test('a window seeded with cached peer info never lays out', () {
      // A monitor window is opened by an earlier layout and seeded with the
      // session's cached peer info; on reconnect it gets a live one too.
      // Laying out there is what turned 4 windows into 8.
      final gate = DisplayLayoutGate();
      expect(gate.shouldLayOut(isCache: true), false);
      expect(gate.shouldLayOut(isCache: false), false);
    });

    test('skip() closes the gate before any peer info arrives', () {
      // A session opened for one specific display must never lay out, even
      // if a live peer info beats its cached seed.
      final gate = DisplayLayoutGate();
      gate.skip();
      expect(gate.shouldLayOut(isCache: false), false);
    });
  });

  group('computeDisplayOpenOrder', () {
    test('no ranks keeps index order', () {
      expect(computeDisplayOpenOrder({}, 4), [0, 1, 2, 3]);
    });

    test('full permutation follows ranks', () {
      // M1(d0)->3, M2(d1)->1, M3(d2)->4, M4(d3)->2
      expect(computeDisplayOpenOrder({0: 3, 1: 1, 2: 4, 3: 2}, 4),
          [1, 3, 0, 2]);
    });

    test('unranked displays go last in index order', () {
      expect(computeDisplayOpenOrder({3: 1}, 4), [3, 0, 1, 2]);
    });

    test('duplicate ranks tie-break by display index', () {
      expect(computeDisplayOpenOrder({1: 2, 2: 2}, 4), [1, 2, 0, 3]);
    });

    test('ranks for displays that no longer exist are ignored', () {
      expect(computeDisplayOpenOrder({7: 1}, 4), [0, 1, 2, 3]);
    });

    test('every display appears exactly once', () {
      final order = computeDisplayOpenOrder({2: 1, 0: 2}, 3);
      expect(order..sort(), [0, 1, 2]);
    });
  });
}
