create or replace function get_rast_tile(
    param_format text,
    param_width integer,
    param_height integer,
    param_srid integer,
    param_bbox text,
    param_schema text,
    param_table text
) returns bytea
    immutable
    parallel safe
    cost 1000
    language plpgsql
as
$$
DECLARE
    var_sql text; var_result raster; var_srid integer;
    var_env geometry; var_env_buf geometry; var_erast raster;
BEGIN
    EXECUTE
        'SELECT ST_MakeEnvelope(' || array_to_string(('{' || param_bbox || '}')::float8[], ',') || ',' || param_srid ||')'
    INTO var_env;

    EXECUTE
        'SELECT ST_Buffer($1, 20);'
    USING var_env
    INTO var_env_buf;

    var_sql :=
            'SELECT srid, ST_AsRaster($4,$5,$6,pixel_types,nodata_values,nodata_values) As erast
            FROM raster_columns
            WHERE r_table_schema = $1 AND r_table_name = $2 AND r_raster_column=$3';

    EXECUTE var_sql INTO var_srid, var_erast
        USING param_schema, param_table, 'rast', var_env, param_width, param_height;

    var_sql :=
        'WITH r AS (SELECT ST_Clip(rast,' ||
        CASE
            WHEN var_srid = param_srid THEN '$7'
            ELSE 'ST_Transform($7,$2)'
            END || ') As rast FROM  ' ||
        quote_ident(param_schema) || '.' ||
        quote_ident(param_table) || '
        WHERE ST_Intersects(rast,' ||
        CASE
            WHEN var_srid = param_srid THEN '$3'
            ELSE 'ST_Transform($3,$2)'
            END || ') limit 15)
        SELECT ST_Clip(ST_Union(rast), $3) As rast
        FROM (SELECT ST_Resample(' ||
        CASE
            WHEN var_srid = param_srid THEN 'rast'
            ELSE 'ST_Transform(rast,$1)'
            END ||
        ',$6,true,''NearestNeighbor'') As rast FROM r) As final';
    EXECUTE var_sql INTO var_result
        USING
            param_srid,
            var_srid,
            var_env,
            param_width,
            param_height,
            var_erast,
            var_env_buf;
    var_sql :=
        'SELECT ST_MapAlgebra($1, $2, ''[rast2]'', ''8BUI''::text, ''FIRST'', ''[rast2]'', NULL::text) rast';
    EXECUTE var_sql INTO var_result
    USING var_erast, var_result;

    IF var_result IS NULL THEN
        var_result := var_erast;
    END IF;

    RETURN
        CASE
            WHEN param_format ILIKE 'image/jpeg' THEN ST_AsJPEG(ST_ColorMap(var_result, 1, 'bluered'))
            ELSE ST_AsPNG(ST_ColorMap(var_result, 1, 'bluered'))
            END;
END;
$$;