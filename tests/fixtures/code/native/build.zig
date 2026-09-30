const std = @import("std");

const Point = struct {
    x: i32,
    y: i32,

    pub fn sum(self: Point) i32 {
        return self.x + self.y;
    }
};

const Mode = enum { debug, release };

const limit = 10;

pub fn main() void {
    std.debug.print("start\n", .{});
}

test "point sum" {
    const point = Point{ .x = 1, .y = 2 };
    try std.testing.expect(point.sum() == 3);
}
