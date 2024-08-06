from fastapi import Depends, APIRouter, Request, Header, HTTPException
from typing import Any, Annotated
from collections import defaultdict
from app.auth import require_admin, User
from app.db import get_session, AsyncSession
from sqlmodel import select
from app.layers.models import Layer
from app.s3.services import get_s3
from aioboto3 import Session as S3Session
from app.config import config
from osgeo import gdal, gdalconst
import os
from fastapi import UploadFile, Form

router = APIRouter()

# In-memory storage for file parts
file_storage = defaultdict(dict)


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


class FilePondUpload(UploadFile):
    filename: str
    file: UploadFile


@router.post("")
async def upload_file(
    request: Request,
    upload_length: int = Header(None, alias="Upload-Length"),
    content_type: str = Header(..., alias="Content-Type"),
    # content_disposition: str = Header(None, alias="Content-Disposition"),
    *,
    # file: UploadFile = File(...),
    # filepond:  = Form(...),
    file: Annotated[UploadFile, Form()],
    # background_tasks: BackgroundTasks,
    user: User = Depends(require_admin),
    session: AsyncSession = Depends(get_session),
    s3: S3Session = Depends(get_s3),
) -> Any:
    """Handle file upload"""

    # Get the filename from the data body
    filename = file.filename
    data = file.file.read()

    try:
        # Create a new object in the database
        # Extract filename into variables. Structure is:
        # {crop}_{watermodel}_{climatemodel}_{scenario}_{variable}_{year}.tif
        try:
            crop, water_model, climate_model, scenario, variable, year = (
                # Remove file extension then split by _
                filename.lower()
                .split(".")[0]
                .split("_")
            )
        except Exception as e:
            print(e)
            raise HTTPException(
                status_code=400,
                detail={
                    "message": "Invalid filename, must be in the format "
                    "{crop}_{watermodel}_{climatemodel}_{scenario}_"
                    "{variable}_{year}.tif",
                },
            )
        print(crop, water_model, climate_model, scenario, variable, year)

        # Query DB to check if the layers exist, if they do, avoid upload
        query = select(Layer).where(
            Layer.crop == crop,
            Layer.water_model == water_model,
            Layer.climate_model == climate_model,
            Layer.scenario == scenario,
            Layer.variable == variable,
            Layer.year == int(year),
        )

        duplicate_layer = await session.exec(query)
        duplicate_layer = duplicate_layer.one_or_none()

        if duplicate_layer:
            raise HTTPException(
                status_code=409,
                detail={
                    "message": f"Layer already exists for {filename}. Delete layer first to re-upload",
                },
            )

        # First convert the file to a COG
        cog_bytes = convert_to_cog_in_memory(data)

        # Get min/max of the raster
        min_val, max_val = get_min_max_of_raster(cog_bytes)

        # Upload the file to S3
        response = await s3.put_object(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{str(filename)}",
            Body=cog_bytes,
        )

        if "ETag" not in response:
            raise HTTPException(
                status_code=500,
                detail="Failed to upload object to S3",
            )

        obj = Layer(
            filename=filename,
            all_parts_received=True,
            crop=crop,
            water_model=water_model,
            climate_model=climate_model,
            scenario=scenario,
            variable=variable,
            year=int(year),
            layer_name=filename.split(".")[0],
            min_value=min_val,
            max_value=max_val,
        )

        session.add(obj)
        await session.commit()
        await session.refresh(obj)

        return obj

    except HTTPException as e:
        raise e
    except Exception as e:
        raise HTTPException(
            status_code=500,
            detail=f"Failed to upload file: {e}",
        )
