//! Inertia of pan and tilt: a moving head accelerates, travels at most at its
//! top speed and brakes in time to stop on the target, instead of jumping.

/// One axis (pan or tilt) in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Axis {
    /// `None` until the first target: a freshly built fixture starts in place.
    pub angle: Option<f32>,
    /// Degrees per second.
    pub velocity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// Degrees per second; 0 = no inertia (follow DMX at once).
    pub max_speed: f32,
    /// Degrees per second squared.
    pub acceleration: f32,
}

impl Axis {
    /// Moves towards `target` for `dt` seconds and returns the new angle.
    pub fn step(&mut self, target: f32, limits: Limits, dt: f32) -> f32 {
        let Some(angle) = self.angle else {
            self.angle = Some(target);
            return target;
        };
        if limits.max_speed <= 0.0 || limits.acceleration <= 0.0 || dt <= 0.0 {
            self.angle = Some(target);
            self.velocity = 0.0;
            return target;
        }
        let error = target - angle;
        // Fastest speed from which we can still brake to a stop at the target.
        let braking = (2.0 * limits.acceleration * error.abs()).sqrt();
        let wanted = error.signum() * braking.min(limits.max_speed);
        let dv = limits.acceleration * dt;
        self.velocity += (wanted - self.velocity).clamp(-dv, dv);
        let mut next = angle + self.velocity * dt;
        // Arriving (or overshooting within one step) ends the move exactly on target.
        if (target - next).signum() != error.signum() || (target - next).abs() < 1e-3 {
            next = target;
            self.velocity = 0.0;
        }
        self.angle = Some(next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits { max_speed: 180.0, acceleration: 720.0 };

    fn run(axis: &mut Axis, target: f32, seconds: f32) -> f32 {
        let dt = 1.0 / 120.0;
        let mut a = 0.0;
        for _ in 0..(seconds / dt) as usize {
            a = axis.step(target, LIMITS, dt);
        }
        a
    }

    #[test]
    fn starts_in_place() {
        let mut axis = Axis::default();
        assert_eq!(axis.step(90.0, LIMITS, 0.01), 90.0);
    }

    #[test]
    fn jump_takes_time_and_respects_top_speed() {
        let mut axis = Axis { angle: Some(0.0), velocity: 0.0 };
        // 180° at 180°/s plus accelerating and braking: clearly more than one second.
        let after_1s = run(&mut axis, 180.0, 1.0);
        assert!((100.0..175.0).contains(&after_1s), "{after_1s}");
        assert!(axis.velocity <= LIMITS.max_speed + 1e-3);
        let done = run(&mut axis, 180.0, 1.0);
        assert_eq!(done, 180.0);
        assert_eq!(axis.velocity, 0.0);
    }

    #[test]
    fn reverses_smoothly() {
        let mut axis = Axis { angle: Some(0.0), velocity: 0.0 };
        run(&mut axis, 100.0, 0.3);
        let v = axis.velocity;
        assert!(v > 0.0);
        // A new target behind us: the head brakes first instead of flipping direction.
        axis.step(-100.0, LIMITS, 1.0 / 120.0);
        assert!(axis.velocity > 0.0 && axis.velocity < v);
    }

    #[test]
    fn zero_speed_means_no_inertia() {
        let mut axis = Axis { angle: Some(0.0), velocity: 0.0 };
        assert_eq!(axis.step(45.0, Limits { max_speed: 0.0, acceleration: 720.0 }, 0.01), 45.0);
    }
}
