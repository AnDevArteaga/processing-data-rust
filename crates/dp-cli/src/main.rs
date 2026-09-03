//! La CLI. Solo lee argumentos, elige el sumidero e imprime resultados.
//! Toda la lógica vive en la librería.

// El paquete se llama `dp-core`, pero en el código los guiones se vuelven
// guiones bajos: `dp_core`.
use dp_core::{
    Config, Envoltura, ErrorDp, LIMITE_CLAVES_POR_DEFECTO, Resumen, Sumidero, SumideroCsv,
    SumideroJson, SumideroNulo, buscar_operacion, catalogo,
};
use std::env;
use std::process;

/// Los argumentos ya interpretados: posicionales, banderas y configuración
/// del job.
struct Argumentos {
    posicionales: Vec<String>,
    json: bool,
    config: Config,
}

impl Argumentos {
    /// Interpretación manual de `--clave=valor`. Cuando la CLI crezca esto se
    /// reemplaza por el crate `clap`, pero conviene ver primero qué es lo que
    /// clap hace por nosotros.
    fn leer() -> Self {
        let mut posicionales = Vec::new();
        let mut json = false;
        let mut config = Config::default();

        // Si el usuario especifica claves a mano, las nuestras por defecto
        // estorban: hay que reemplazarlas, no añadirse a ellas.
        let mut claves_dadas = false;
        let mut emails_dados = false;
        let mut telefonos_dados = false;

        for argumento in env::args().skip(1) {
            // `split_once` divide en el PRIMER '=', así que un valor que
            // contenga '=' sigue llegando entero.
            let (bandera, valor) = match argumento.split_once('=') {
                Some((b, v)) => (b.to_string(), Some(v.to_string())),
                None => (argumento.clone(), None),
            };

            match (bandera.as_str(), valor) {
                ("--json", _) => json = true,

                ("--sin-dedup", _) => {
                    config.claves.clear();
                    claves_dadas = true;
                }

                ("--clave", Some(v)) => {
                    if !claves_dadas {
                        config.claves.clear();
                        claves_dadas = true;
                    }
                    // Se puede repetir la bandera o separar por comas:
                    // --clave=email,telefono
                    config.claves.extend(trocear(&v));
                }

                ("--email", Some(v)) => {
                    if !emails_dados {
                        config.emails.clear();
                        emails_dados = true;
                    }
                    config.emails.extend(trocear(&v));
                }

                ("--telefono", Some(v)) => {
                    if !telefonos_dados {
                        config.telefonos.clear();
                        telefonos_dados = true;
                    }
                    config.telefonos.extend(trocear(&v));
                }

                ("--limite", Some(v)) => {
                    // Un límite ilegible es culpa del usuario, pero no vale la
                    // pena un error del motor: avisamos y seguimos con el tope
                    // por defecto.
                    match v.parse::<usize>() {
                        Ok(n) => config.limite_claves = n,
                        Err(_) => eprintln!(
                            "aviso: --limite={v} no es un numero, uso {LIMITE_CLAVES_POR_DEFECTO}"
                        ),
                    }
                }

                (otra, _) if otra.starts_with("--") => {
                    eprintln!("aviso: bandera desconocida '{otra}', la ignoro");
                }

                _ => posicionales.push(argumento),
            }
        }

        Argumentos {
            posicionales,
            json,
            config,
        }
    }
}

/// "email, telefono" -> ["email", "telefono"], descartando lo vacío.
/// Así `--email=` (sin valor) sirve para decir "ninguna columna es email".
fn trocear(valor: &str) -> Vec<String> {
    valor
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn ayuda() {
    println!("uso: dp <operacion> <entrada> [salida] [banderas]");
    println!();
    println!("operaciones disponibles:");
    for op in catalogo() {
        println!("  {:<14} {}", op.nombre(), op.descripcion());
    }
    println!();
    println!("la salida es .csv o .json segun la extension que le des");
    println!();
    println!("banderas:");
    println!("  --clave=a,b     columnas que forman la clave de deduplicacion");
    println!("  --sin-dedup     no deduplicar: deja pasar todas las filas");
    println!("  --email=a,b     columnas a normalizar y validar como email");
    println!("  --telefono=a,b  columnas a normalizar como telefono");
    println!("  --limite=N      tope de claves unicas en memoria");
    println!("  --json          imprime el resultado como una sola linea JSON");
    println!();
    println!("por defecto: --clave=email --email=email --telefono=telefono");
    println!("             --limite={LIMITE_CLAVES_POR_DEFECTO}");
    println!();
    println!("codigos de salida:");
    println!("  0  todo bien");
    println!("  1  fallo interno: el job se puede reintentar");
    println!("  2  datos del cliente invalidos: no reintentar");
}

fn main() {
    let args = Argumentos::leer();

    if args.posicionales.is_empty() {
        ayuda();
        return;
    }

    match ejecutar(&args) {
        Ok(resumen) => {
            if args.json {
                // `unwrap` aquí es aceptable: serializar una struct de números
                // no puede fallar. Si fallara, sería un bug nuestro, no un
                // error de datos.
                let envoltura = Envoltura::exito(resumen);
                println!("{}", serde_json::to_string(&envoltura).unwrap());
            } else {
                println!();
                println!("{}", serde_json::to_string_pretty(&resumen).unwrap());
            }
        }
        Err(error) => {
            if args.json {
                let envoltura = Envoltura::fallo(&error);
                // A stderr: stdout queda reservado para el resultado del job.
                eprintln!("{}", serde_json::to_string(&envoltura).unwrap());
            } else {
                eprintln!("[{}] {error}", error.codigo());
                for causa in error.cadena_de_causas() {
                    eprintln!("  causa: {causa}");
                }
                if error.es_reintentable() {
                    eprintln!("  (fallo interno: el job se puede reintentar)");
                } else {
                    eprintln!("  (dato de entrada invalido: no reintentar el job)");
                }
            }

            // El código de salida es lo unico que el orquestador puede leer
            // sin interpretar texto.
            process::exit(error.codigo_salida());
        }
    }
}

fn ejecutar(args: &Argumentos) -> Result<Resumen, ErrorDp> {
    let nombre_op = &args.posicionales[0];
    let operacion = buscar_operacion(nombre_op, args.config.clone())
        .ok_or_else(|| ErrorDp::OperacionDesconocida(nombre_op.clone()))?;

    let entrada = args
        .posicionales
        .get(1)
        .cloned()
        .unwrap_or_else(|| "data/clientes.csv".to_string());

    let mut sumidero: Box<dyn Sumidero> = if !operacion.produce_archivo() {
        Box::new(SumideroNulo)
    } else {
        let salida = args
            .posicionales
            .get(2)
            .cloned()
            .unwrap_or_else(|| "data/salida.csv".to_string());
        if !args.json {
            println!("Salida:    {salida}");
        }
        if salida.ends_with(".json") {
            Box::new(SumideroJson::nuevo(&salida)?)
        } else {
            Box::new(SumideroCsv::nuevo(&salida)?)
        }
    };

    if !args.json {
        println!("Operacion: {}", operacion.nombre());
        println!("Entrada:   {entrada}");
        if args.config.claves.is_empty() {
            println!("Dedup:     desactivada");
        } else {
            println!("Dedup:     {}", args.config.claves.join(" + "));
        }
    }

    operacion.ejecutar(&entrada, &mut *sumidero)
}
