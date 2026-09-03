//! Un reloj que no avanza solo, para los tests.

use chrono::{DateTime, Duration, Utc};
use dp_dominio::Reloj;
use std::sync::Mutex;

/// Reloj controlado a mano.
///
/// Sin esto, probar «un job cuyo plazo venció» obligaría a dormir de verdad.
/// Con esto, se adelanta el reloj cinco minutos y el test tarda microsegundos.
pub struct RelojFijo {
    // `Mutex` y no `Cell` porque `Reloj` exige `Sync`: el reloj se comparte
    // entre tareas igual que el resto de la infraestructura.
    ahora: Mutex<DateTime<Utc>>,
}

impl RelojFijo {
    pub fn nuevo(inicio: DateTime<Utc>) -> Self {
        RelojFijo {
            ahora: Mutex::new(inicio),
        }
    }

    pub fn adelantar(&self, cuanto: Duration) {
        let mut guardia = self.ahora.lock().expect("el mutex del reloj no se envenena");
        *guardia += cuanto;
    }
}

impl Default for RelojFijo {
    fn default() -> Self {
        RelojFijo::nuevo(Utc::now())
    }
}

impl Reloj for RelojFijo {
    fn ahora(&self) -> DateTime<Utc> {
        *self.ahora.lock().expect("el mutex del reloj no se envenena")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_reloj_fijo_no_avanza_por_si_solo() {
        let reloj = RelojFijo::default();
        let primera = reloj.ahora();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(reloj.ahora(), primera);
    }

    #[test]
    fn adelantar_mueve_el_reloj_exactamente_lo_pedido() {
        let inicio = Utc::now();
        let reloj = RelojFijo::nuevo(inicio);

        reloj.adelantar(Duration::minutes(5));
        assert_eq!(reloj.ahora(), inicio + Duration::minutes(5));
    }
}
