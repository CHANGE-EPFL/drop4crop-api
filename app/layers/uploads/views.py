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
from fastapi import UploadFile, Form
from app.layers.utils import convert_to_cog_in_memory, get_min_max_of_raster

router = APIRouter()

# In-memory storage for file parts
file_storage = defaultdict(dict)


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
    filename = file.filename.lower()  # Ensure everything is lowercase
    data = file.file.read()

    try:
        # Create a new object in the database
        # Extract filename into variables. Structure is:
        # {crop}_{watermodel}_{climatemodel}_{scenario}_{variable}_{year}.tif
        try:
            # Remove file extension then split by _
            split_filename = filename.lower().split(".")[0].split("_")
            is_crop_variable = False
            if len(split_filename) == 6:
                # To manage normal layers
                crop, water_model, climate_model, scenario, variable, year = (
                    split_filename
                )
            elif len(split_filename) == 2:
                # To manage crop variables
                crop, variable = split_filename
                is_crop_variable = True
            else:
                # Anything else is unsupported
                raise Exception("Invalid filename")

        except Exception as e:
            print(e)
            raise HTTPException(
                status_code=400,
                detail={
                    "message": "Invalid filename, must be either in the "
                    "format {crop}_{watermodel}_{climatemodel}_{scenario}_"
                    "{variable}_{year}.tif, or {crop}_{crop_variable}.tif",
                },
            )

        # Query DB to check if the layers exist, if they do, avoid upload
        if is_crop_variable:
            query = select(Layer).where(
                Layer.crop == crop,
                Layer.variable == variable,
                Layer.is_crop_specific,
            )
        else:
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
                    "message": (
                        f"Layer already exists for {filename}. "
                        "Delete layer first to re-upload"
                    ),
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

        if is_crop_variable:
            # Create a new layer object
            obj = Layer(
                filename=filename,
                all_parts_received=True,
                crop=crop,
                variable=variable,
                min_value=min_val,
                max_value=max_val,
                layer_name=filename.split(".")[0],  # A bit naive, I'm sorry
                is_crop_specific=True,
            )
        else:
            obj = Layer(
                filename=filename,
                all_parts_received=True,
                crop=crop,
                water_model=water_model,
                climate_model=climate_model,
                scenario=scenario,
                variable=variable,
                year=int(year),
                layer_name=filename.split(".")[0],  # Again, sorry...
                min_value=min_val,
                max_value=max_val,
                is_crop_specific=False,
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
