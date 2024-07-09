from app.layers.models import (
    Layer,
    LayerRead,
    LayerReadAuthenticated,
    LayerCreate,
    LayerUpdate,
)
from app.db import get_session, AsyncSession
from fastapi import (
    Depends,
    APIRouter,
    Query,
    Response,
    HTTPException,
    BackgroundTasks,
)
from uuid import UUID, uuid4
from app.crud import CRUD
from itertools import product
import httpx
from app.config import config

router = APIRouter()


crud = CRUD(Layer, LayerRead, LayerCreate, LayerUpdate)


async def get_count(
    response: Response,
    filter: str = Query(None),
    range: str = Query(None),
    sort: str = Query(None),
    session: AsyncSession = Depends(get_session),
):
    count = await crud.get_total_count(
        response=response,
        sort=sort,
        range=range,
        filter=filter,
        session=session,
    )

    return count


async def get_all_authenticated(
    filter: str = Query(None),
    sort: str = Query(None),
    range: str = Query(None),
    session: AsyncSession = Depends(get_session),
):

    res = await crud.get_model_data(
        sort=sort,
        range=range,
        filter=filter,
        session=session,
    )

    read_layers = []
    for layer in res:
        obj = LayerReadAuthenticated.model_validate(layer)
        async with httpx.AsyncClient() as client:
            # Get layer styling information from geoserver
            response = await client.get(
                f"{config.GEOSERVER_URL}/rest/layers/"
                f"{config.GEOSERVER_WORKSPACE}"
                f":{layer.layer_name}.json",
                auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
            )
            if response.status_code == 200:
                obj.style_name = response.json()["layer"]["defaultStyle"][
                    "name"
                ]
                obj.created_at = response.json()["layer"]["dateCreated"]

        read_layers.append(obj)

    return read_layers


async def get_one(
    layer_id: UUID,
    session: AsyncSession = Depends(get_session),
) -> LayerReadAuthenticated:
    res = await crud.get_model_by_id(model_id=layer_id, session=session)
    res = LayerReadAuthenticated.model_validate(res)
    # Get layer information from geoserver
    async with httpx.AsyncClient() as client:
        # Get layer styling information from geoserver
        response = await client.get(
            f"{config.GEOSERVER_URL}/rest/layers/{config.GEOSERVER_WORKSPACE}"
            f":{res.layer_name}.json",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
        if response.status_code == 200:
            res.style_name = response.json()["layer"]["defaultStyle"]["name"]
            res.created_at = response.json()["layer"]["dateCreated"]

    if not res:
        raise HTTPException(
            status_code=404, detail=f"ID: {layer_id} not found"
        )
    return res


async def create_one(
    data: dict,
    session: AsyncSession,
    background_tasks: BackgroundTasks,
) -> Layer:
    """Create a single layer

    To be used in both create one and create many endpoints
    """

    obj = Layer.model_validate(data)

    session.add(obj)

    await session.commit()
    await session.refresh(obj)

    return obj


async def update_one(
    layer_id: UUID,
    layer_update: LayerUpdate,
    session: AsyncSession = Depends(get_session),
) -> Layer:
    """Update a single layer"""

    obj = await crud.get_model_by_id(model_id=layer_id, session=session)

    update_data = layer_update.model_dump(exclude_unset=True)

    # If the style_name is updated, update the style in geoserver but first
    # check that the style exists before trying
    if "style_name" in update_data:
        async with httpx.AsyncClient() as client:
            response = await client.get(
                f"{config.GEOSERVER_URL}/rest/styles/{update_data['style_name']}.json",
                auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
            )
            if response.status_code > 299:
                raise HTTPException(
                    status_code=response.status_code,
                    detail=response.text,
                )

            # Update the style in geoserver
            response = await client.put(
                f"{config.GEOSERVER_URL}/rest/layers/{config.GEOSERVER_WORKSPACE}"
                f":{obj.layer_name}",
                auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
                json={
                    "layer": {
                        "defaultStyle": {
                            "name": update_data["style_name"],
                        }
                    }
                },
            )
            if response.status_code > 299:
                raise HTTPException(
                    status_code=response.status_code,
                    detail=response.text,
                )

    obj.sqlmodel_update(update_data)

    session.add(obj)
    await session.commit()
    await session.refresh(obj)

    obj = await get_one(layer_id, session)

    return obj


def get_layers() -> dict[tuple[str, str, str, str, str, int], LayerRead]:
    crops = [
        "barley",
        "potato",
        "rice",
        "soy",
        "sugarcane",
        "wheat",
    ]
    water_models = [
        # "cwatm",
        # "h08",
        # "lpjml",
        # "matsiro",
        "pcr-globwb",
        "watergap2",
    ]
    climate_models = [
        "gfdl-esm2m",
        "hadgem2-es",
        "ipsl-cm5a-lr",
        "miroc5",
    ]
    scenarios = [
        "historical",
        "rcp26",
        "rcp60",
        "rcp85",
    ]
    variables = [
        # "vwc_sub",
        # "vwcb_sub",
        # "vwcg_sub",
        # "vwcg_perc",
        # "vwcb_perc",
        "wf",
        "wfb",
        "wfg",
        "etb",
        "etg",
        "rb",
        "rg",
        "wdb",
        "wdg",
    ]
    years = list(range(2000, 2100, 10))

    combinations = product(
        crops, water_models, climate_models, scenarios, variables, years
    )
    layers = {
        (
            crop,
            water_model,
            climate_model,
            scenario,
            variable,
            year,
        ): LayerRead(
            id=uuid4(),
            crop=crop,
            water_model=water_model,
            climate_model=climate_model,
            scenario=scenario,
            variable=variable,
            year=year,
            layer_name=f"{crop}_{water_model}_{climate_model}_{scenario}_{variable}_{year}".lower(),
        )
        for crop, water_model, climate_model, scenario, variable, year in combinations
    }
    return layers


async def create_layers(session: AsyncSession):
    layers = get_layers()

    # Upload all results to the database
    for key, value in layers.items():
        layer_dict = value.model_dump()
        layer_dict.pop("id")
        obj = Layer.model_validate(layer_dict)

        # print("OBJECT", obj)
        session.add(obj)

    await session.commit()

    return
