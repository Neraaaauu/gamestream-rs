use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, InputEvent, KeyCode, RelativeAxisCode, UinputAbsSetup,
};
use tracing::{info, warn};

use crate::protocol::{GamepadEvent, KeyEvent, MouseEvent};
use crate::InputError;

pub struct LinuxInputInjector {
    mouse: VirtualDevice,
    keyboard: VirtualDevice,
    gamepad: Option<VirtualDevice>,
    last_mouse_buttons: u16,
}

impl LinuxInputInjector {
    pub fn new() -> Result<Self, InputError> {
        info!("Inicializando inyector de entrada virtual en Linux (/dev/uinput)...");

        // 1. Ratón Virtual
        let mut mouse_keys = AttributeSet::<KeyCode>::new();
        mouse_keys.insert(KeyCode::BTN_LEFT);
        mouse_keys.insert(KeyCode::BTN_RIGHT);
        mouse_keys.insert(KeyCode::BTN_MIDDLE);
        mouse_keys.insert(KeyCode::BTN_SIDE);
        mouse_keys.insert(KeyCode::BTN_EXTRA);

        let mut mouse_rel = AttributeSet::<RelativeAxisCode>::new();
        mouse_rel.insert(RelativeAxisCode::REL_X);
        mouse_rel.insert(RelativeAxisCode::REL_Y);
        mouse_rel.insert(RelativeAxisCode::REL_WHEEL);
        mouse_rel.insert(RelativeAxisCode::REL_HWHEEL);

        let mouse = VirtualDevice::builder()
            .map_err(|e| InputError::UinputInit(format!("Error abriendo /dev/uinput para mouse: {e}")))?
            .name("Gamestream Virtual Mouse")
            .with_keys(&mouse_keys)
            .map_err(|e| InputError::UinputInit(format!("Error configurando botones de mouse: {e}")))?
            .with_relative_axes(&mouse_rel)
            .map_err(|e| InputError::UinputInit(format!("Error configurando ejes de mouse: {e}")))?
            .build()
            .map_err(|e| InputError::UinputInit(format!("Error creando ratón virtual: {e}")))?;

        info!("Ratón virtual creado exitosamente.");

        // 2. Teclado Virtual
        let mut kb_keys = AttributeSet::<KeyCode>::new();
        for code in 1..255 {
            kb_keys.insert(KeyCode::new(code));
        }

        let keyboard = VirtualDevice::builder()
            .map_err(|e| InputError::UinputInit(format!("Error abriendo /dev/uinput para teclado: {e}")))?
            .name("Gamestream Virtual Keyboard")
            .with_keys(&kb_keys)
            .map_err(|e| InputError::UinputInit(format!("Error configurando teclas: {e}")))?
            .build()
            .map_err(|e| InputError::UinputInit(format!("Error creando teclado virtual: {e}")))?;

        info!("Teclado virtual creado exitosamente.");

        // 3. Gamepad Virtual Xbox 360
        let gamepad = match Self::create_virtual_gamepad() {
            Ok(g) => {
                info!("Gamepad virtual Microsoft X-Box 360 creado exitosamente.");
                Some(g)
            }
            Err(e) => {
                warn!("No se pudo crear gamepad virtual ({e}); streaming de mando no disponible");
                None
            }
        };

        Ok(Self {
            mouse,
            keyboard,
            gamepad,
            last_mouse_buttons: 0,
        })
    }

    fn create_virtual_gamepad() -> Result<VirtualDevice, InputError> {
        let mut pad_keys = AttributeSet::<KeyCode>::new();
        pad_keys.insert(KeyCode::BTN_SOUTH); // A
        pad_keys.insert(KeyCode::BTN_EAST);  // B
        pad_keys.insert(KeyCode::BTN_NORTH); // X
        pad_keys.insert(KeyCode::BTN_WEST);  // Y
        pad_keys.insert(KeyCode::BTN_TL);    // LB
        pad_keys.insert(KeyCode::BTN_TR);    // RB
        pad_keys.insert(KeyCode::BTN_SELECT);// Back
        pad_keys.insert(KeyCode::BTN_START); // Start
        pad_keys.insert(KeyCode::BTN_MODE);  // Guide / Xbox button
        pad_keys.insert(KeyCode::BTN_THUMBL);// LS click
        pad_keys.insert(KeyCode::BTN_THUMBR);// RS click

        let stick_info = AbsInfo::new(0, -32768, 32767, 16, 128, 0);
        let trigger_info = AbsInfo::new(0, 0, 255, 0, 0, 0);
        let dpad_info = AbsInfo::new(0, -1, 1, 0, 0, 0);

        VirtualDevice::builder()
            .map_err(|e| InputError::UinputInit(format!("Fallo uinput gamepad: {e}")))?
            .name("Microsoft X-Box 360 pad")
            .with_keys(&pad_keys)
            .map_err(|e| InputError::UinputInit(format!("Fallo botones gamepad: {e}")))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, stick_info))
            .map_err(|e| InputError::UinputInit(format!("Fallo eje ABS_X: {e}")))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, stick_info))
            .map_err(|e| InputError::UinputInit(format!("Fallo eje ABS_Y: {e}")))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_RX, stick_info))
            .map_err(|e| InputError::UinputInit(format!("Fallo eje ABS_RX: {e}")))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_RY, stick_info))
            .map_err(|e| InputError::UinputInit(format!("Fallo eje ABS_RY: {e}")))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Z, trigger_info))
            .map_err(|e| InputError::UinputInit(format!("Fallo eje ABS_Z: {e}")))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_RZ, trigger_info))
            .map_err(|e| InputError::UinputInit(format!("Fallo eje ABS_RZ: {e}")))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_HAT0X, dpad_info))
            .map_err(|e| InputError::UinputInit(format!("Fallo eje ABS_HAT0X: {e}")))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_HAT0Y, dpad_info))
            .map_err(|e| InputError::UinputInit(format!("Fallo eje ABS_HAT0Y: {e}")))?
            .build()
            .map_err(|e| InputError::UinputInit(format!("Error construyendo gamepad: {e}")))
    }

    pub fn inject_mouse(&mut self, event: &MouseEvent) -> Result<(), InputError> {
        let mut events = Vec::with_capacity(6);

        if event.dx != 0 {
            events.push(InputEvent::new(evdev::EventType::RELATIVE.0, RelativeAxisCode::REL_X.0, event.dx as i32));
        }
        if event.dy != 0 {
            events.push(InputEvent::new(evdev::EventType::RELATIVE.0, RelativeAxisCode::REL_Y.0, event.dy as i32));
        }
        if event.wheel != 0 {
            events.push(InputEvent::new(evdev::EventType::RELATIVE.0, RelativeAxisCode::REL_WHEEL.0, event.wheel as i32));
        }

        // Evaluar cambios en botones de ratón
        let changed = self.last_mouse_buttons ^ event.buttons;
        if changed != 0 {
            if changed & 1 != 0 {
                let val = if event.buttons & 1 != 0 { 1 } else { 0 };
                events.push(InputEvent::new(evdev::EventType::KEY.0, KeyCode::BTN_LEFT.0, val));
            }
            if changed & 2 != 0 {
                let val = if event.buttons & 2 != 0 { 1 } else { 0 };
                events.push(InputEvent::new(evdev::EventType::KEY.0, KeyCode::BTN_RIGHT.0, val));
            }
            if changed & 4 != 0 {
                let val = if event.buttons & 4 != 0 { 1 } else { 0 };
                events.push(InputEvent::new(evdev::EventType::KEY.0, KeyCode::BTN_MIDDLE.0, val));
            }
            self.last_mouse_buttons = event.buttons;
        }

        if !events.is_empty() {
            self.mouse.emit(&events).map_err(|e| InputError::EmitFailed(e.to_string()))?;
        }

        Ok(())
    }

    pub fn inject_keyboard(&mut self, event: &KeyEvent) -> Result<(), InputError> {
        let key_ev = InputEvent::new(evdev::EventType::KEY.0, event.scancode, event.state as i32);
        self.keyboard.emit(&[key_ev]).map_err(|e| InputError::EmitFailed(e.to_string()))
    }

    pub fn inject_gamepad(&mut self, event: &GamepadEvent) -> Result<(), InputError> {
        let pad = match self.gamepad.as_mut() {
            Some(p) => p,
            None => return Ok(()),
        };

        let mut events = Vec::with_capacity(20);

        // Ejes analógicos y Gatillos (LT / RT)
        events.push(InputEvent::new(evdev::EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_X.0, event.lx as i32));
        events.push(InputEvent::new(evdev::EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Y.0, event.ly as i32));
        events.push(InputEvent::new(evdev::EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_RX.0, event.rx as i32));
        events.push(InputEvent::new(evdev::EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_RY.0, event.ry as i32));
        events.push(InputEvent::new(evdev::EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Z.0, event.lt as i32));
        events.push(InputEvent::new(evdev::EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_RZ.0, event.rt as i32));

        // Cruceta D-Pad (ABS_HAT0X y ABS_HAT0Y)
        let hat_x = if (event.buttons & 0x0008) != 0 { 1 } else if (event.buttons & 0x0004) != 0 { -1 } else { 0 };
        let hat_y = if (event.buttons & 0x0002) != 0 { 1 } else if (event.buttons & 0x0001) != 0 { -1 } else { 0 };
        events.push(InputEvent::new(evdev::EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_HAT0X.0, hat_x));
        events.push(InputEvent::new(evdev::EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_HAT0Y.0, hat_y));

        // Botones de acción estándar XInput
        let btn = |code: KeyCode, mask: u16| {
            let val = if (event.buttons & mask) != 0 { 1 } else { 0 };
            InputEvent::new(evdev::EventType::KEY.0, code.0, val)
        };

        events.push(btn(KeyCode::BTN_SOUTH, 0x1000)); // A
        events.push(btn(KeyCode::BTN_EAST, 0x2000));  // B
        events.push(btn(KeyCode::BTN_NORTH, 0x4000)); // X
        events.push(btn(KeyCode::BTN_WEST, 0x8000));  // Y
        events.push(btn(KeyCode::BTN_TL, 0x0100));    // LB (Bumper Izquierdo)
        events.push(btn(KeyCode::BTN_TR, 0x0200));    // RB (Bumper Derecho)
        events.push(btn(KeyCode::BTN_SELECT, 0x0020));// Back / View
        events.push(btn(KeyCode::BTN_START, 0x0010)); // Start / Menu
        events.push(btn(KeyCode::BTN_THUMBL, 0x0040));// LS (Click Stick Izq)
        events.push(btn(KeyCode::BTN_THUMBR, 0x0080));// RS (Click Stick Der)
        events.push(btn(KeyCode::BTN_MODE, 0x0400));  // Botón Guía Xbox

        pad.emit(&events).map_err(|e| InputError::EmitFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_input_creation_and_injection() {
        let mut injector = match LinuxInputInjector::new() {
            Ok(i) => i,
            Err(e) => {
                println!("Omitiendo prueba si no hay permisos de /dev/uinput: {e}");
                return;
            }
        };

        let mouse_ev = MouseEvent { dx: 15, dy: -10, buttons: 1, wheel: 0 };
        assert!(injector.inject_mouse(&mouse_ev).is_ok());

        let key_ev = KeyEvent { scancode: 30, state: 1, _pad: 0 };
        assert!(injector.inject_keyboard(&key_ev).is_ok());

        let pad_ev = GamepadEvent {
            pad_id: 0,
            buttons: 1,
            lt: 128,
            rt: 255,
            lx: 10000,
            ly: -10000,
            rx: 5000,
            ry: -5000,
            _pad: 0,
        };
        assert!(injector.inject_gamepad(&pad_ev).is_ok());
    }
}
