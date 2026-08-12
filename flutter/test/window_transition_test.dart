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
}
