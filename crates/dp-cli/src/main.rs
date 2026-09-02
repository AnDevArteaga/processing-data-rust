//! La CLI. Solo se encarga de leer argumentos, elegir el sumidero e imprimir
//! resultados. Toda la lógica vive en la librería.

// El paquete se llama `dp-core`, pero en el código los guiones se vuelven
// guiones bajos: `dp_core`.
use dp_core::{
    ErrorDp, Resultado, Sumidero, SumideroCsv, SumideroJson, SumideroNulo, buscar_operacion,
    catalogo,
};
use std::env;
use std::error::Error as StdError;
use std::process;

fn ayuda() {
    println!("uso: dp <operacion> <entrada> [salida]");
    println!();
    println!("operaciones disponibles:");
    for op in catalogo() {
        println!("  {:<14} {}", op.nombre(), op.descripcion());
    }
    println!();
    println!("la salida es .csv o .json segun la extension que le des");
}

fn main() {
    if let Err(error) = ejecutar() {
        // El código y la clasificación son lo que consumirían la API y el
        // sistema de alertas, no el texto del mensaje.
        eprintln!("[{}] {error}", error.codigo());

        let mut causa = error.source();
        while let Some(actual) = causa {
            eprintln!("  causa: {actual}");
            causa = actual.source();
        }

        if error.es_culpa_del_cliente() {
            eprintln!("  (dato de entrada invalido: no reintentar el job)");
        } else {
            eprintln!("  (fallo interno: el job se puede reintentar)");
        }

        process::exit(1);
    }
}

fn ejecutar() -> Resultado<()> {
    let argumentos: Vec<String> = env::args().skip(1).collect();

    if argumentos.is_empty() {
        ayuda();
        return Ok(());
    }

    let nombre_op = &argumentos[0];
    let operacion = buscar_operacion(nombre_op)
        .ok_or_else(|| ErrorDp::OperacionDesconocida(nombre_op.clone()))?;

    let entrada = argumentos
        .get(1)
        .cloned()
        .unwrap_or_else(|| "data/clientes.csv".to_string());

    let mut sumidero: Box<dyn Sumidero> = if !operacion.produce_archivo() {
        Box::new(SumideroNulo)
    } else {
        let salida = argumentos
            .get(2)
            .cloned()
            .unwrap_or_else(|| "data/salida.csv".to_string());
        println!("Salida:    {salida}");
        if salida.ends_with(".json") {
            Box::new(SumideroJson::nuevo(&salida)?)
        } else {
            Box::new(SumideroCsv::nuevo(&salida)?)
        }
    };

    println!("Operacion: {}", operacion.nombre());
    println!("Entrada:   {entrada}");

    let resumen = operacion.ejecutar(&entrada, &mut *sumidero)?;

    println!();
    println!("{}", serde_json::to_string_pretty(&resumen)?);

    Ok(())
}
