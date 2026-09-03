//! Cálculo de créditos.
//!
//! La sección 16 del PDF exige medir antes de poner precio. Estos números no
//! son inventados: salen del benchmark del motor sobre un CSV real de 150 MB
//! con 2,5 millones de filas, donde deduplicar por email tomó 4,8 segundos de
//! CPU y 189 MB de memoria, y no deduplicar tomó 1,4 segundos y 3,9 MB.

// `self::` es la ruta a este mismo módulo. El submódulo de abajo existe solo
// para agrupar las constantes de precio; el glob las trae a este ámbito.
use self::modelo_de_costo::*;
use dp_core::Resumen;

/// Constantes de precio, agrupadas para que se vean juntas y se puedan
/// calibrar sin buscarlas por el archivo.
mod modelo_de_costo {
    /// Filas que cubre un crédito en `csv.clean`.
    ///
    /// 2,5 millones de filas cuestan 10 créditos con este divisor, y consumen
    /// unos 5 segundos de CPU. A 10 créditos por 5 segundos, un plan Pro de
    /// 30.000 créditos compra unas 4 horas de CPU al mes.
    pub const FILAS_POR_CREDITO: u64 = 250_000;

    /// Bytes por fila observados en el archivo de prueba: 150 MB / 2,5 M.
    /// Solo se usa para *estimar* antes de leer el archivo.
    pub const BYTES_POR_FILA_ESTIMADOS: u64 = 60;

    /// Recargo por deduplicar, en porcentaje.
    ///
    /// Deduplicar triplicó el tiempo y multiplicó la memoria por cincuenta. La
    /// memoria es lo que fija el tamaño del worker, así que es el costo que
    /// hay que reflejar.
    pub const RECARGO_DEDUP_PORCENTAJE: u64 = 50;

    /// Una inspección cuesta un crédito fijo, a propósito.
    ///
    /// Es la vista previa: si es barata, el cliente la usa antes de pagar el
    /// job completo, descubre que su archivo tiene problemas y no nos pide un
    /// reembolso. Nos ahorra soporte.
    pub const CREDITOS_INSPECCION: u64 = 1;

    /// Ningún job cuesta cero: aunque falle rápido, ocupó un worker.
    pub const MINIMO: u64 = 1;
}

fn aplicar_recargo(base: u64, deduplica: bool) -> u64 {
    if deduplica {
        // División entera al final para no perder precisión en el camino.
        base + (base * RECARGO_DEDUP_PORCENTAJE) / 100
    } else {
        base
    }
}

/// Cuánto reservar al crear el job, cuando solo conocemos el tamaño.
///
/// Se reserva antes de encolar porque si no, un cliente con un crédito podría
/// encolar mil jobs a la vez: todos pasarían la validación antes de que el
/// primero termine y descontara.
pub fn creditos_estimados(operacion: &str, bytes: u64, deduplica: bool) -> u64 {
    match operacion {
        "csv.inspect" => CREDITOS_INSPECCION,
        "csv.clean" => {
            let filas = bytes / BYTES_POR_FILA_ESTIMADOS;
            // `div_ceil` redondea hacia arriba: media unidad de trabajo se
            // cobra como una. Es lo correcto y evita el caso de cero.
            let base = filas.div_ceil(FILAS_POR_CREDITO).max(MINIMO);
            aplicar_recargo(base, deduplica)
        }
        // Una operación que no conocemos no se puede tasar, y devolver cero
        // sería regalarla. La API rechaza antes las operaciones desconocidas,
        // así que esto es solo una red de seguridad.
        _ => MINIMO,
    }
}

/// Cuánto cobrar al terminar, ya con las filas reales contadas.
pub fn creditos_reales(operacion: &str, resumen: &Resumen, deduplica: bool) -> u64 {
    match operacion {
        "csv.inspect" => CREDITOS_INSPECCION,
        "csv.clean" => {
            let base = resumen.leidas.div_ceil(FILAS_POR_CREDITO).max(MINIMO);
            aplicar_recargo(base, deduplica)
        }
        _ => MINIMO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El caso medido: 150 MB, 2,5 millones de filas.
    #[test]
    fn el_archivo_de_referencia_cuesta_lo_esperado() {
        let bytes = 150 * 1024 * 1024;

        // Sin deduplicar: 10 créditos por 2,5 M de filas.
        assert_eq!(creditos_estimados("csv.clean", bytes, false), 11);
        // Deduplicando: recargo del 50%.
        assert_eq!(creditos_estimados("csv.clean", bytes, true), 16);
    }

    #[test]
    fn la_inspeccion_cuesta_siempre_un_credito() {
        assert_eq!(creditos_estimados("csv.inspect", 0, false), 1);
        assert_eq!(creditos_estimados("csv.inspect", 5_000_000_000, true), 1);
    }

    /// Ningún job puede salir gratis: siempre ocupó un worker.
    #[test]
    fn un_archivo_diminuto_cuesta_el_minimo() {
        assert_eq!(creditos_estimados("csv.clean", 10, false), 1);
        assert_eq!(creditos_estimados("csv.clean", 0, false), 1);

        let resumen = Resumen {
            leidas: 1,
            ..Resumen::default()
        };
        assert_eq!(creditos_reales("csv.clean", &resumen, false), 1);
    }

    /// El cobro real se basa en filas contadas, no en bytes estimados. Un CSV
    /// con columnas anchas cuesta menos de lo estimado, y eso está bien: la
    /// reserva es un techo, no el precio.
    #[test]
    fn el_cobro_real_puede_ser_menor_que_la_reserva() {
        let bytes = 150 * 1024 * 1024;
        let reservado = creditos_estimados("csv.clean", bytes, true);

        // El archivo tenía filas anchas: solo 500.000 filas en 150 MB.
        let resumen = Resumen {
            leidas: 500_000,
            ..Resumen::default()
        };
        let cobrado = creditos_reales("csv.clean", &resumen, true);

        assert!(
            cobrado < reservado,
            "cobrado {cobrado} deberia ser menor que reservado {reservado}"
        );
        assert_eq!(cobrado, 3);
    }

    #[test]
    fn el_recargo_de_deduplicacion_es_del_cincuenta_por_ciento() {
        let resumen = Resumen {
            leidas: 2_000_000,
            ..Resumen::default()
        };
        assert_eq!(creditos_reales("csv.clean", &resumen, false), 8);
        assert_eq!(creditos_reales("csv.clean", &resumen, true), 12);
    }

    #[test]
    fn el_redondeo_es_siempre_hacia_arriba() {
        let resumen = Resumen {
            leidas: FILAS_POR_CREDITO + 1,
            ..Resumen::default()
        };
        assert_eq!(creditos_reales("csv.clean", &resumen, false), 2);
    }
}
