from osgeo import gdal, gdalconst
import os
from typing import AsyncGenerator
import aioboto3


def sort_styles(style_list):
    return sorted(style_list, key=lambda x: x["value"])


def generate_grayscale_style(min_value, max_value, num_segments=10):
    step = (max_value - min_value) / num_segments
    grayscale_style = []

    for i in range(num_segments):
        value = min_value + i * step
        grey_value = int(255 * (i / (num_segments - 1)))
        grayscale_style.append(
            {
                "value": value,
                "red": grey_value,
                "green": grey_value,
                "blue": grey_value,
                "opacity": 255,
                "label": round(value, 4),
            }
        )

    return grayscale_style


def get_min_max_of_raster(
    input_bytes: bytes,
) -> tuple[float, float]:
    """Get the min and max values of a raster"""

    # Create an in-memory file from the input bytes
    input_filename = "/vsimem/input.tif"
    gdal.FileFromMemBuffer(input_filename, input_bytes)

    # Open the file with gdal, calculate statistics, then return min max
    ds = gdal.Open(input_filename, gdalconst.GA_ReadOnly)
    band = ds.GetRasterBand(1)
    min_val, max_val = band.ComputeRasterMinMax()
    return min_val, max_val


def convert_to_cog_in_memory(
    input_bytes: bytes,
) -> bytes:
    """Convert in-memory GeoTIFF to Cloud Optimized GeoTIFF using GDAL"""

    print("Converting to COG")
    # Create an in-memory file from the input bytes
    input_filename = "/vsimem/input.tif"
    gdal.FileFromMemBuffer(input_filename, input_bytes)

    # Output in-memory file for the COG
    output_filename = "/vsimem/output-cog.tif"
    options = gdal.TranslateOptions(
        format="COG", creationOptions=["OVERVIEWS=NONE"]
    )
    gdal.Translate(output_filename, input_filename, options=options)

    # Read the in-memory COG file back to a byte array
    output_ds = gdal.VSIFOpenL(output_filename, "rb")
    gdal.VSIFSeekL(output_ds, 0, os.SEEK_END)
    size = gdal.VSIFTellL(output_ds)
    gdal.VSIFSeekL(output_ds, 0, os.SEEK_SET)
    cog_bytes = gdal.VSIFReadL(1, size, output_ds)
    gdal.VSIFCloseL(output_ds)

    # Clean up in-memory files
    gdal.Unlink(input_filename)
    gdal.Unlink(output_filename)
    print("COG conversion successful")

    return cog_bytes


async def get_file_chunk(
    bucket_name: str,
    key: str,
    chunk_length: int,
    s3: aioboto3.Session,
) -> AsyncGenerator[bytes, None]:
    """Async generator to get file chunk."""

    head = await s3.head_object(Bucket=bucket_name, Key=key)
    content_length = head["ContentLength"]

    for offset in range(0, content_length, chunk_length):
        end = min(offset + chunk_length - 1, content_length - 1)
        s3_file = await s3.get_object(
            Bucket=bucket_name, Key=key, Range=f"bytes={offset}-{end}"
        )

        async with s3_file["Body"] as stream:
            yield await stream.read()
